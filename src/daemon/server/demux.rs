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

use super::resolution::work_root_for;
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
    tracing::debug!(
        kind = event.kind.as_u16(),
        id = %&event.id.to_hex()[..8],
        from = %crate::util::pubkey_short(&event.pubkey.to_hex()),
        "incoming event"
    );
    let env = crate::fabric::RawEnvelope::Nostr(event.clone());
    // Expand the hosted set to include live transient session pubkeys.
    // This makes `is_self` (Profile/Status self-skip), the routing gate
    // (`hosted.contains(&m.to_pubkey)`), and the sender admission check
    // (`hosted.contains(&sender_pk)`) all recognize session-signed events.
    let hosted: Vec<String> = {
        let mut h = state.hosted_pubkeys();
        h.extend(state.live_session_pubkeys());
        // Durable ordinal pubkeys (issue #47) are local identities too: a mention
        // p-tagged to e.g. `smith1` must be recognized as self so the routing gate
        // and self-skip treat it like a hosted agent, not a foreign peer.
        h.extend(state.with_store(|s| s.list_identity_pubkeys().unwrap_or_default()));
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
        let kind = event.kind.as_u16();
        if state.first_sight(&event.id.to_hex()) {
            // Status heartbeats (kind:30315) fire every 30 s — too noisy for info.
            let is_heartbeat = kind == 30315;
            if is_heartbeat {
                tracing::debug!(kind, id = %&event.id.to_hex()[..8], "first-sight");
            } else {
                tracing::info!(kind, id = %&event.id.to_hex()[..8], "first-sight");
            }
            derive_and_emit_tail_events(state, &de, &hosted, now);
            if event.kind.as_u16() == crate::fabric::nip29::wire::KIND_CHAT {
                if let DomainEvent::ChatMessage(ref chat) = de {
                    if let Some(ref mentioned_pk) = chat.mentioned_pubkey {
                        let st = state.clone();
                        let mentioned_pk = mentioned_pk.clone();
                        let project = chat.project.clone();
                        tracing::info!(
                            mentioned_pk = %crate::util::pubkey_short(&mentioned_pk),
                            project = %project,
                            "dispatching offline-agent-mention handler"
                        );
                        tokio::spawn(async move {
                            handle_offline_agent_mention(&st, &mentioned_pk, &project).await;
                        });
                    }
                }
            }
        } else {
            tracing::debug!(
                kind = event.kind.as_u16(),
                id = %&event.id.to_hex()[..8],
                "duplicate delivery — skipped"
            );
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
            tracing::info!(
                event_id = %&event.id.to_hex()[..8],
                parent = %op.parent,
                child = %op.child_h,
                "dispatching orchestration handler"
            );
            let st = state.clone();
            let ev = event.clone();
            tokio::spawn(async move {
                handle_orchestration(&st, &ev, op).await;
            });
        } else if is_management_command_for_backend(state, event) {
            tracing::info!(
                event_id = %&event.id.to_hex()[..8],
                "dispatching management command handler"
            );
            let st = state.clone();
            let ev = event.clone();
            tokio::spawn(async move {
                handle_management_command(&st, &ev).await;
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
            .any(|rec| {
                rec.agent_pubkey == mentioned_pk
                    && s.is_session_joined_channel(&rec.session_id, project)
                        .unwrap_or(rec.channel_h == project)
            })
    });
    if has_alive {
        tracing::debug!(
            mentioned_pk = %crate::util::pubkey_short(mentioned_pk),
            project,
            "agent already has alive session — skipping spawn"
        );
        return;
    }

    // Resolve the mentioned pubkey to (agent_slug, ordinal). It may be a base
    // agent key (ordinal 0, in the keystore) OR a durable ordinal key (issue #47)
    // recorded in the `identities` table.
    let edge = crate::config::edge_home();
    let local_agents = crate::identity::list_local_agent_details(&edge);
    let (agent_slug, ordinal, base_pubkey) =
        match local_agents.iter().find(|a| a.pubkey == mentioned_pk) {
            Some(a) => (a.slug.clone(), 0u32, mentioned_pk.to_string()),
            None => match state.with_store(|s| s.get_identity(mentioned_pk).ok().flatten()) {
                Some(idn) => (idn.agent_slug, idn.ordinal, idn.base_pubkey),
                None => return,
            },
        };

    // Resume vs fresh: if this identity previously ran in this channel and left a
    // bound native session, RESUME that harness (restores its conversation);
    // otherwise spawn fresh with the exact ordinal.
    let bound = state.with_store(|s| {
        s.resolve_identity_for_channel(&base_pubkey, project)
            .ok()
            .flatten()
    });
    if let Some(route) = bound.filter(|r| !r.native_id.is_empty()) {
        tracing::info!(
            agent = %route.agent_slug,
            project,
            native_id = %route.native_id,
            "resuming bound native session"
        );
        if let Err(e) =
            crate::tmux::resume_agent(state, &agent_slug, project, &route.native_id).await
        {
            tracing::warn!(agent = %agent_slug, project, error = %e, "session resume failed — falling through to fresh spawn");
        } else {
            return;
        }
    }

    let is_member =
        state.with_store(|s| s.is_channel_member(project, mentioned_pk).unwrap_or(false));
    if !is_member {
        let (_, _, members) = state.provider.fetch_group_state(project).await;
        if !members.contains(mentioned_pk) {
            if ordinal == 0 {
                // Base agent must already be a member to be mentioned here.
                tracing::debug!(agent = %agent_slug, project, "base agent not a member of channel — skipping spawn");
                return;
            }
            tracing::info!(agent = %agent_slug, ordinal, project, "provisioning ordinal pubkey into channel via mgmt key");
            if !state
                .provider
                .grant_member_confirmed(project, mentioned_pk)
                .await
                .is_confirmed()
            {
                tracing::warn!(agent = %agent_slug, ordinal, project, "mgmt-key add_member was not confirmed — skipping spawn");
                return;
            }
        }
    }

    let work_root = state.with_store(|s| work_root_for(s, project));
    let has_path = state.with_store(|s| s.project_root(&work_root).ok().flatten().is_some());
    if !has_path {
        tracing::warn!(agent = %agent_slug, work_root = %work_root, project, "no local project root found — cannot spawn");
        return;
    }

    let group_arg = Some(project);
    tracing::info!(
        agent = %agent_slug,
        ordinal,
        project,
        work_root = %work_root,
        "spawning agent on mention"
    );
    match crate::tmux::spawn_agent(
        state,
        &agent_slug,
        &work_root,
        Vec::new(),
        None,
        group_arg,
        None,
        Some(ordinal),
    )
    .await
    {
        Ok(pane_id) => {
            tracing::info!(agent = %agent_slug, pane_id = %pane_id, project, "agent spawned successfully")
        }
        Err(e) => tracing::warn!(agent = %agent_slug, project, error = %e, "agent spawn failed"),
    }
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
            for channel in &s.channels {
                // The unified Status replaces the old presence heartbeat, so
                // first-sight of a (pubkey, session, channel) here is the peer
                // "joined" signal for that channel.
                let key = (
                    s.agent.pubkey.clone(),
                    s.session_id.as_str().to_string(),
                    channel.clone(),
                );
                let is_new = {
                    let mut map = state.peer_sessions.lock().unwrap();
                    if !map.contains_key(&key) {
                        map.insert(
                            key.clone(),
                            PeerTracked {
                                first_seen: now,
                                project: channel.clone(),
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
                        project: channel.clone(),
                        agent: s.agent.slug.clone(),
                        host: s.host.clone(),
                        session: s.session_id.as_str().to_string(),
                        rel_cwd: s.rel_cwd.clone(),
                    });
                }

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
                        project: channel.clone(),
                        agent: s.agent.slug.clone(),
                        text: s.title.clone(),
                        active: s.busy,
                    });
                }
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
