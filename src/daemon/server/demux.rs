//! Relay demux pipeline extracted from `server.rs` (issue #12, EPIC-server-001).
//!
//! One relay subscription feeds every hosted agent. `spawn_demux` drains the
//! notification stream; `handle_incoming` materializes each event once and
//! derives real-time `TailEvent`s via `derive_and_emit_tail_events`. The two
//! async side-channels (`handle_offline_agent_mention`, `handle_orchestration`)
//! are dispatched off the demux loop.
//!
//! Pure function movement — behavior is byte-identical to the pre-split file.
//! `spawn_demux` and `handle_orchestration` are `pub(super)` because the parent
//! module calls them (the accept-loop bootstrap and the channels_create local
//! fast-path); everything else is private to this module.

use super::*;

pub(super) fn spawn_demux(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut notifications = state.transport.notifications();
        loop {
            let ev: Option<Event> = match notifications.recv().await {
                Ok(RelayPoolNotification::Event { event, .. }) => Some(*event),
                Ok(RelayPoolNotification::Message {
                    message: RelayMessage::Event { event, .. },
                    ..
                }) => Some(event.into_owned()),
                Ok(_) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(_) => None,
            };
            if let Some(event) = ev {
                handle_incoming(&state, &event);
            }
        }
    });
}

/// Decode one event and apply it. Multi-agent aware: "me" is the SET of hosted
/// local pubkeys; a mention routes by `to_pubkey` to that agent's sessions only.
///
/// Thin dispatch to `provider.materialize` (Phase 5), then derives TailEvents
/// from the domain event using the in-memory tracking maps.
fn handle_incoming(state: &Arc<DaemonState>, event: &Event) {
    eprintln!(
        "[demux] incoming kind:{} id:{} from:{}",
        event.kind.as_u16(),
        &event.id.to_hex()[..8],
        crate::util::pubkey_short(&event.pubkey.to_hex()),
    );
    let env = crate::fabric::RawEnvelope::Nostr(event.clone());
    // Expand the hosted set to include live transient session pubkeys.
    // This makes `is_self` (Profile/Status self-skip), the routing gate
    // (`hosted.contains(&m.to_pubkey)`), and the sender admission check
    // (`hosted.contains(&sender_pk)`) all recognize session-signed events.
    let hosted: Vec<String> = {
        let mut h = state.hosted_pubkeys();
        h.extend(state.live_session_pubkeys());
        h.sort_unstable();
        h.dedup();
        h
    };
    let now = now_secs();
    // ALWAYS materialize: store writes are idempotent, and re-deliveries are
    // load-bearing — a refreshed subscription replays stored events, which is
    // how a NEW session receives mentions that predate it.
    let outcome = state.with_store(|s| state.provider.materialize(&env, &hosted, now, s));

    // The relay pool notifies once PER MATCHING SUBSCRIPTION (scope filters ×
    // live sessions), so the same event reaches here many times. The tail
    // broadcast is NOT idempotent — emit only on first sight of the event id.
    // Spawn-on-mention also runs inside first_sight so we attempt at most once
    // per run; has_alive check in the handler covers daemon-restart idempotency.
    if let Some(de) = outcome.tail {
        if state.first_sight(&event.id.to_hex()) {
            derive_and_emit_tail_events(state, &de, &hosted, now);
            if event.kind.as_u16() == crate::fabric::nip29::wire::KIND_CHAT {
                if let DomainEvent::ChatMessage(ref chat) = de {
                    if let Some(ref mentioned_pk) = chat.mentioned_pubkey {
                        let st = state.clone();
                        let mentioned_pk = mentioned_pk.clone();
                        let project = chat.project.clone();
                        tokio::spawn(async move {
                            handle_offline_agent_mention(&st, &mentioned_pk, &project).await;
                        });
                    }
                }
            }
        }
    }
    if outcome.wake_mentions {
        crate::tmux::ring_doorbells(state.clone());
    }

    // When a kind:39002 membership snapshot arrives, ensure we have a group
    // subscription for any group a local agent just joined. `ensure_subscription`
    // is idempotent for already-subscribed groups.
    if event.kind.as_u16() == crate::fabric::nip29::wire::KIND_GROUP_MEMBERS {
        if let Some(project) = crate::fabric::nip29::nostr_tag(event, "d") {
            let local_pks = state.hosted_pubkeys();
            let is_member = event.tags.iter().any(|t| {
                let s = t.as_slice();
                s.first().map(String::as_str) == Some("p")
                    && s.get(1).map(|pk| local_pks.contains(pk)).unwrap_or(false)
            });
            if is_member {
                let st = state.clone();
                let proj = project.to_string();
                tokio::spawn(async move {
                    let _ = ensure_subscription(&st, &proj).await;
                });
            }
        }
    }

    // Subgroup orchestration (issue #3): a kind:9 carrying the add-agents op tag
    // asks backends to provision agent roles into a child group. Parse tags ONLY
    // (prose is ignored); dispatch the async handler off the demux loop. Durable
    // idempotency lives inside the handler, not `first_sight` (which is in-memory
    // and would respawn agents after a daemon restart).
    if event.kind.as_u16() == crate::fabric::nip29::wire::KIND_CHAT {
        if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(event) {
            let st = state.clone();
            let ev = event.clone();
            tokio::spawn(async move {
                handle_orchestration(&st, &ev, op).await;
            });
        }
    }
}

/// Spawn a local agent that was p-tagged in a kind:9 message but had no alive
/// session. Idempotency: `first_sight` prevents duplicate spawns within a run;
/// `has_alive` prevents re-spawn across restarts when the previous spawn registered.
/// Delivery: `rpc_session_start` calls `ensure_subscription`, which triggers a
/// relay replay of recent kind:9 events; those are re-materialized against the
/// now-alive session and delivered via `ring_doorbells`.
async fn handle_offline_agent_mention(state: &Arc<DaemonState>, mentioned_pk: &str, project: &str) {
    let has_alive = state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .any(|rec| rec.agent_pubkey == mentioned_pk && rec.route_scope() == project)
    });
    if has_alive {
        return;
    }

    let edge = crate::config::edge_home();
    let local_agents = crate::identity::list_local_agent_details(&edge);
    let agent_slug = match local_agents.into_iter().find(|a| a.pubkey == mentioned_pk) {
        Some(a) => a.slug,
        None => return,
    };
    let is_member = state.with_store(|s| s.is_group_member(project, mentioned_pk).unwrap_or(false));
    if !is_member {
        let (_, _, members) = state.provider.fetch_group_state(project).await;
        if !members.contains(mentioned_pk) {
            eprintln!("[spawn-on-mention] {agent_slug} not a member of {project}, skip");
            return;
        }
    }

    let work_root = state.with_store(|s| {
        s.work_root_for_scope(project)
            .unwrap_or_else(|_| project.to_string())
    });
    let has_path = state.with_store(|s| s.get_project_path(&work_root).ok().flatten().is_some());
    if !has_path {
        eprintln!("[spawn-on-mention] no local path for {work_root}, cannot spawn");
        return;
    }

    let group_arg: Option<&str> = if project != work_root {
        Some(project)
    } else {
        None
    };
    eprintln!("[spawn-on-mention] spawning {agent_slug} into {project} (work_root={work_root})");
    match crate::tmux::spawn_agent(
        state,
        &agent_slug,
        &work_root,
        Vec::new(),
        None,
        group_arg,
        None,
    )
    .await
    {
        Ok(pane_id) => eprintln!("[spawn-on-mention] {agent_slug} spawned pane={pane_id}"),
        Err(e) => eprintln!("[spawn-on-mention] spawn failed: {e:#}"),
    }
}

/// React to a subgroup add-agents orchestration event: authorize the signer,
/// provision the agents addressed to THIS backend (mint identity, publish
/// kind:0, add as member), and spawn each agent's harness into the child group.
/// Best-effort and idempotent (durable `processed_orchestration` guard).
pub(super) async fn handle_orchestration(
    state: &Arc<DaemonState>,
    event: &Event,
    op: crate::fabric::nip29::orchestration::AddAgentsOp,
) {
    use crate::fabric::nip29::orchestration::{adds_for_backend, is_authorized};

    let event_id = event.id.to_hex();
    // Only agents addressed to THIS backend's identity concern us. (Checked BEFORE
    // claiming so a foreign event never burns this backend's idempotency slot.)
    let Some(backend_pk) = state.backend_pubkey().map(|s| s.to_string()) else {
        return;
    };
    let mine: Vec<_> = adds_for_backend(&op.adds, &backend_pk)
        .into_iter()
        .cloned()
        .collect();
    if mine.is_empty() {
        return;
    }

    // Authorize: the signer must be an admin of the parent (where authority
    // lives) or of the child. Fail closed on fetch error (treat as unauthorized).
    // Done BEFORE the claim so a transient fetch failure doesn't permanently mark
    // the event processed.
    let signer = event.pubkey.to_hex();
    let parent_roles = state.provider.fetch_group_roles(&op.parent).await;
    let authorized = is_authorized(&parent_roles, &signer) || {
        let child_roles = state.provider.fetch_group_roles(&op.child_h).await;
        is_authorized(&child_roles, &signer)
    };
    if !authorized {
        if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
            eprintln!(
                "[daemon] orchestration {} from {} ignored: signer is not an admin of {} or {}",
                &event_id[..event_id.len().min(8)],
                crate::util::pubkey_short(&signer),
                op.parent,
                op.child_h
            );
        }
        return;
    }

    // Guard against a parent-admin directing spawns into an UNRELATED group: if
    // the child's relay metadata already declares a parent, it must match. A
    // brand-new child whose 39000 hasn't echoed yet (None) is allowed through.
    if let Some(declared) = state.provider.fetch_group_parent(&op.child_h).await {
        if declared != op.parent {
            eprintln!(
                "[daemon] orchestration {}: child {} declares parent {declared:?}, not {:?}; refusing",
                &event_id[..event_id.len().min(8)],
                op.child_h,
                op.parent
            );
            return;
        }
    }

    // Atomically CLAIM the event now that all pre-checks passed. Only the first
    // of the relay's duplicate deliveries wins; the rest return here. Placed
    // AFTER auth/parent checks (transient-safe) but BEFORE any mutating work, so
    // concurrent tasks never race on identity minting or member-adds.
    if !state.with_store(|s| s.try_claim_orchestration(&event_id, now_secs())) {
        return;
    }

    // Subscribe + own the child so we receive its state and can manage it.
    state.with_store(|s| {
        s.mark_group_owned(&op.child_h, now_secs()).ok();
    });
    let _ = ensure_subscription(state, &op.child_h).await;

    let edge = config::edge_home();
    for target in &mine {
        let slug = &target.slug;
        let id = match crate::identity::load_or_create(&edge, slug, now_secs()) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("[daemon] orchestration: minting agent {slug:?} failed: {e:#}");
                state.with_store(|s| s.unclaim_orchestration(&event_id));
                return;
            }
        };
        let agent_pk = id.pubkey_hex();
        log_nip29_role_decision(
            &op.child_h,
            &agent_pk,
            "member",
            "handle_orchestration target agent durable pubkey",
        );

        // Publish the durable agent's kind:0 identity card.
        let profile = DomainEvent::Profile(crate::domain::Profile {
            agent: crate::domain::AgentRef::new(agent_pk.clone(), slug.clone()),
            host: state.host.clone(),
            owners: state.owners.clone(),
            is_backend: false,
        });
        let _ = state.provider.publish(&profile, &id.keys).await;

        // Add the durable agent pubkey as a MEMBER (never admin) of the child, and
        // CONFIRM it landed in the relay's roster. The relay acks a put-user on
        // receipt but only APPLIES the membership if the author is an admin at
        // apply-time — and this backend's own admin grant (published moments
        // earlier by the orchestrator) may still be propagating. So trust-but-
        // verify: re-issue + read back the 39002 roster a few times before giving
        // up. Gate the spawn on a CONFIRMED member-add (a live harness whose
        // events the relay rejects is worse than no harness).
        let mut confirmed = false;
        for attempt in 0..12u32 {
            let outcome = state
                .provider
                .nip29_add_member_outcome(&op.child_h, &agent_pk)
                .await;
            let (_, _, members) = state.provider.fetch_group_state(&op.child_h).await;
            // Two independent confirmations, EITHER suffices:
            //  (a) the relay's published 39002 roster lists the agent, or
            //  (b) a RE-issued add (attempt > 0) is accepted as benign — for
            //      nip29.f7z.io phrases this as "all targets are members
            //      already", i.e. the relay's authoritative in-memory membership
            //      already holds the agent. Relying on (a) alone deadlocks when the
            //      relay's 39002 replaceable is stale (a same-second created_at
            //      collision can freeze the public roster even though membership is
            //      applied), because every retry is then rejected-as-redundant and
            //      the agent never reappears in the readback. (b) breaks that tie.
            let relay_confirms_member =
                members.contains(&agent_pk) || (attempt > 0 && outcome.is_applied());
            if relay_confirms_member {
                confirmed = true;
                break;
            }
            if outcome.is_rejected() {
                break;
            }
            // Evenly spaced (not bursty) so two backends confirming at once don't
            // starve the relay's async apply queue.
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        }
        if !confirmed {
            eprintln!(
                "[daemon] orchestration: member-add for agent {slug:?} in {} not confirmed on the \
                 relay after retries; skipping spawn (will retry on re-delivery)",
                op.child_h
            );
            state.with_store(|s| s.unclaim_orchestration(&event_id));
            return;
        }
        state.with_store(|s| {
            s.upsert_group_member(&op.child_h, &agent_pk, "member", now_secs())
                .ok();
        });

        // Spawn the harness in the PARENT project's working directory but scoped
        // to the child channel (TENEX_EDGE_CHANNEL). The spawned session's
        // session-start path adds its derived session pubkey to the child group.
        match crate::tmux::spawn_agent(
            state,
            slug,
            &op.parent,
            Vec::new(),
            None,
            Some(&op.child_h),
            None,
        )
        .await
        {
            Ok(pane) => {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] orchestration: spawned agent {slug:?} into {} (pane {pane})",
                        op.child_h
                    );
                }
            }
            Err(e) => {
                eprintln!("[daemon] orchestration: spawn agent {slug:?} failed: {e:#}");
            }
        }
    }
    // The claim taken above is the durable "processed" marker; nothing more to do.
}

/// Convert a decoded `DomainEvent` into zero or more `TailEvent`s and emit them.
/// Skip is_self events for presence/status (local lifecycle handled by RPC emitters).
fn derive_and_emit_tail_events(
    state: &Arc<DaemonState>,
    de: &DomainEvent,
    hosted: &[String],
    now: u64,
) {
    match de {
        DomainEvent::Proposal(_) => {
            // Proposals are surfaced through the threads read model (the rpc
            // records them as canonical messages); no tail line is derived from
            // the raw inbound event.
        }
        DomainEvent::Status(s) => {
            // Skip own status — local turn/status is tracked by Turn RPC events.
            if hosted.contains(&s.agent.pubkey) {
                return;
            }
            // The unified Status replaces the old presence heartbeat, so
            // first-sight of a (pubkey, project) here is the peer "joined" signal.
            let key = (s.agent.pubkey.clone(), s.project.clone());
            let is_new = {
                let mut map = state.peer_sessions.lock().unwrap();
                if !map.contains_key(&key) {
                    map.insert(
                        key.clone(),
                        PeerTracked {
                            first_seen: now,
                            project: s.project.clone(),
                            slug: s.agent.slug.clone(),
                            host: s.host.clone(),
                        },
                    );
                    true
                } else {
                    false
                }
            };
            if is_new {
                state.emit_tail(TailEvent::Join {
                    ts: now,
                    project: s.project.clone(),
                    agent: s.agent.slug.clone(),
                    host: s.host.clone(),
                    session: s.agent.pubkey.clone(),
                    rel_cwd: s.rel_cwd.clone(),
                });
            }

            // Dedup by (author_pubkey, group_id): all sessions of a durable
            // agent in one project sign with the same key and occupy the same
            // replaceable slot, so per-agent/group dedup is the correct unit.
            let key = (s.agent.pubkey.clone(), s.project.clone());
            let cur = (s.title.clone(), s.busy);
            let should_emit = {
                let mut map = state.last_status.lock().unwrap();
                if map.get(&key) != Some(&cur) {
                    map.insert(key, cur);
                    true
                } else {
                    false
                }
            };
            if should_emit {
                state.emit_tail(TailEvent::Status {
                    ts: now,
                    project: s.project.clone(),
                    agent: s.agent.slug.clone(),
                    text: s.title.clone(),
                    active: s.busy,
                });
            }
        }
        DomainEvent::Profile(pf) => {
            let is_new = {
                let mut set = state.seen_profiles.lock().unwrap();
                set.insert(pf.agent.pubkey.clone())
            };
            if is_new {
                state.emit_tail(TailEvent::Profile {
                    ts: now,
                    agent: pf.agent.slug.clone(),
                    host: pf.host.clone(),
                    pubkey: pf.agent.pubkey.clone(),
                });
            }
        }
        DomainEvent::ChatMessage(chat) => {
            // Local publishes emit their own outbound tail line in rpc_chat_write.
            if hosted.contains(&chat.from.pubkey) {
                return;
            }
            let from_slug = if chat.from.slug.is_empty() {
                pubkey_short(&chat.from.pubkey)
            } else {
                chat.from.slug.clone()
            };
            let to = chat
                .mentioned_pubkey
                .as_deref()
                .map(pubkey_short)
                .unwrap_or_else(|| "project-chat".to_string());
            state.emit_tail(TailEvent::Msg {
                ts: now,
                project: chat.project.clone(),
                from: from_slug,
                from_session: None,
                to,
                to_session: None,
                body: chat.body.chars().take(200).collect(),
            });
        }
        DomainEvent::Activity(_) => {
            // Activity events are not emitted on the tail (they're durable
            // narrative, not real-time transitions).
        }
    }
}
