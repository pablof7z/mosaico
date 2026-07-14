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
//! module calls them (the accept-loop bootstrap and the channel_create local
//! fast-path); everything else is private to this module.

use super::*;

mod chat_ops;
mod offline_mention;
mod route_reaction;

/// Proactively fetch + cache the `kind:0` for any of `pubkeys` we do not already
/// have a name for. Called on every inbound event (a peer newly seen in a
/// 3900x/chat/status) and once at startup for the identities we already know
/// (owners, hosted agents). Known identities are filtered out cheaply and
/// synchronously — they never spawn a task or touch the network — and concurrent
/// duplicate deliveries of the same event collapse to ONE in-flight fetch per
/// pubkey via the `warming` guard. `who` therefore never has to warm: the cache
/// is populated as pubkeys are observed, and it renders names from the cache.
pub(in crate::daemon::server) fn warm_profiles(state: &Arc<DaemonState>, pubkeys: Vec<String>) {
    let to_fetch = claim_pubkeys_to_warm(state, pubkeys);
    if to_fetch.is_empty() {
        return;
    }
    let st = state.clone();
    tokio::spawn(async move {
        for pk in &to_fetch {
            let _ = crate::profile::resolve_name(&st, pk).await;
        }
        // Release the in-flight claims; a fetch that failed (offline relay) is thus
        // retried the next time the pubkey is observed rather than being wedged.
        let mut guard = st.warming.lock().unwrap();
        for pk in &to_fetch {
            guard.remove(pk);
        }
    });
}

/// The synchronous half of [`warm_profiles`]: reduce `pubkeys` to the ones worth a
/// relay fetch and claim them in the in-flight `warming` set. A pubkey is dropped
/// when it is empty, already has a cached name, or is already being fetched — so a
/// known identity never hits the network and duplicate deliveries never stack up.
fn claim_pubkeys_to_warm(state: &Arc<DaemonState>, pubkeys: Vec<String>) -> Vec<String> {
    // A cache miss (no row, or a row with no resolved name) is the only reason to
    // hit the relay; everything already named is skipped.
    let missing = state.with_store(|s| {
        pubkeys
            .into_iter()
            .filter(|pk| !pk.is_empty())
            .filter(|pk| {
                s.get_profile(pk)
                    .ok()
                    .flatten()
                    .map(|p| p.name.is_empty())
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>()
    });
    // Collapse concurrent duplicates: claim each pubkey; a fetch already in flight
    // for it keeps ownership until it completes.
    let mut guard = state.warming.lock().unwrap();
    missing
        .into_iter()
        .filter(|pk| guard.insert(pk.clone()))
        .collect()
}

/// Every identity a raw event references: its author plus all `p`-tagged pubkeys
/// (channel members on a 39001/39002, mention targets on chat). These are the
/// pubkeys whose `kind:0` we want cached so they render by name.
fn referenced_pubkeys(event: &Event) -> Vec<String> {
    let mut refs = vec![event.pubkey.to_hex()];
    refs.extend(event.tags.iter().filter_map(|t| {
        let s = t.as_slice();
        (s.first().map(String::as_str) == Some("p"))
            .then(|| s.get(1).cloned())
            .flatten()
    }));
    refs
}

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
    // Expand the hosted set to include configured offline targets and every
    // known local session pubkey.
    // This makes `is_self` (Profile/Status self-skip), the routing gate
    // (`hosted.contains(&m.to_pubkey)`), and the sender admission check
    // (`hosted.contains(&sender_pk)`) all recognize session-signed events.
    let hosted: Vec<String> = {
        let mut h = state.hosted_pubkeys();
        h.extend(crate::identity::list_local_pubkeys(
            &crate::config::edge_home(),
        ));
        h.extend(state.with_store(|s| s.list_local_session_pubkeys().unwrap_or_default()));
        h.sort_unstable();
        h.dedup();
        h
    };
    let now = now_secs();
    // ALWAYS materialize: store writes are idempotent, and re-deliveries are
    // load-bearing — a refreshed subscription replays stored events, which is
    // how a NEW session receives mentions that predate it.
    let outcome = state.with_store(|s| state.provider.materialize(&env, &hosted, now, s));

    // Proactively resolve the kind:0 of every identity this event just surfaced
    // (author + p-tagged members/mentions), so a peer seen for the first time in a
    // 3900x/chat/status is named without waiting for a turn to warm the cache.
    warm_profiles(state, referenced_pubkeys(event));

    // The relay pool notifies once PER MATCHING SUBSCRIPTION (scope filters ×
    // live sessions), so the same event reaches here many times. The tail
    // broadcast is NOT idempotent — emit only on first sight of the event id.
    // first_sight avoids redundant claims within one process; the durable
    // event+recipient claim covers daemon-restart idempotency.
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
                    if offline_mention::dispatch_all(state, &event.id.to_hex(), chat, &hosted) {
                        let st = state.clone();
                        let ev = event.clone();
                        tokio::spawn(async move {
                            route_reaction::publish_eye_reaction(&st, &ev).await;
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
        crate::session_host::ring_doorbells(state.clone());
    }

    // When a kind:39002 membership snapshot arrives, ensure we have a group
    // subscription for any group a local agent just joined. `ensure_subscription`
    // is idempotent for already-subscribed groups.
    if event.kind.as_u16() == crate::fabric::nip29::wire::KIND_GROUP_MEMBERS {
        if let Some(channel) = crate::fabric::nip29::nostr_tag(event, "d") {
            let local_pks = state.hosted_pubkeys();
            let is_member = event.tags.iter().any(|t| {
                let s = t.as_slice();
                s.first().map(String::as_str) == Some("p")
                    && s.get(1).map(|pk| local_pks.contains(pk)).unwrap_or(false)
            });
            if is_member {
                let st = state.clone();
                let proj = channel.to_string();
                tokio::spawn(async move {
                    let _ = ensure_subscription(&st, &proj).await;
                });
            }
        }
    }

    chat_ops::dispatch(state, event);
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
                // first-sight of a (pubkey, channel) here is the peer
                // "joined" signal for that channel.
                let key = (s.agent.pubkey.clone(), channel.clone());
                let is_new = {
                    let mut map = state.peer_sessions.lock().unwrap();
                    if !map.contains_key(&key) {
                        map.insert(
                            key.clone(),
                            PeerTracked {
                                first_seen: now,
                                channel: channel.clone(),
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
                        channel: channel.clone(),
                        agent: s.agent.slug.clone(),
                        host: s.host.clone(),
                        session: s.agent.pubkey.clone(),
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
                        channel: channel.clone(),
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
            // Local publishes emit their own outbound tail line in rpc_channel_send.
            if hosted.contains(&chat.from.pubkey) {
                return;
            }
            let from_slug = if chat.from.slug.is_empty() {
                pubkey_short(&chat.from.pubkey)
            } else {
                chat.from.slug.clone()
            };
            let to = if chat.mentioned_pubkeys.is_empty() {
                "channel-chat".to_string()
            } else {
                chat.mentioned_pubkeys
                    .iter()
                    .map(|pubkey| pubkey_short(pubkey))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            state.emit_tail(TailEvent::Msg {
                ts: now,
                channel: chat.channel.clone(),
                from: from_slug,
                to,
                body: chat.body.chars().take(200).collect(),
            });
        }
        DomainEvent::Activity(_) => {
            // Activity events are not emitted on the tail (they're durable
            // narrative, not real-time transitions).
        }
        DomainEvent::Reaction(_) => {
            // Reactions never reach the tail (materialize() sets tail=None), and
            // even if one did it is passive awareness with no real-time surface.
        }
    }
}

#[cfg(test)]
#[path = "demux/tests.rs"]
mod tests;
