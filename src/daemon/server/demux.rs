//! Relay demux pipeline extracted from `server.rs` (issue #12, EPIC-server-001).
//!
//! One relay subscription feeds every hosted agent. `spawn_demux` drains the
//! notification stream; incoming events are materialized once and
//! derives real-time `TailEvent`s via `derive_and_emit_tail_events`. The two
//! async side-channels (`handle_offline_agent_mention`, `handle_orchestration`)
//! are dispatched off the demux loop.
//!
//! Pure function movement — behavior is byte-identical to the pre-split file.
//! `spawn_demux` and `handle_orchestration` are `pub(super)` because the parent
//! module calls them (the accept-loop bootstrap and the channel_create local
//! fast-path); everything else is private to this module.

use super::*;

mod attachments;
mod chat_ops;
mod inbound_dispatch;
mod offline_mention;
mod profile_cache;
mod route_reaction;

pub(in crate::daemon::server) use profile_cache::{refetch_missing_profiles, warm_profiles};

pub(in crate::daemon::server) fn drive_offline_mention_retries(state: &Arc<DaemonState>) {
    offline_mention::drive_retries(state);
}

pub(in crate::daemon::server) fn dispatch_offline_mentions(
    state: &Arc<DaemonState>,
    event_id: &str,
    chat: &crate::domain::ChatMessage,
    owned_targets: &[String],
) -> bool {
    offline_mention::dispatch_all(state, event_id, chat, owned_targets)
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
    let mut events = state
        .nmp
        .take_materialization_events()
        .expect("NMP materialization stream has one daemon owner");
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            inbound_dispatch::dispatch(&state, &event);
        }
    });
}

fn finish_incoming(
    state: &Arc<DaemonState>,
    event: &Event,
    outcome: crate::fabric::MaterializationOutcome,
    hosted: Vec<String>,
    now: u64,
    first_sight: bool,
) {
    // Resolve newly surfaced identities without waiting for a turn to warm them.
    warm_profiles(state, referenced_pubkeys(event));
    super::subscriptions::reconcile_after_group_state_event(state, event, first_sight);

    // NMP can deliver once per matching observation (scope filters × live
    // sessions), so the same event reaches here many times. The tail
    // broadcast is NOT idempotent — emit only on first sight of the event id.
    // first_sight avoids redundant claims within one process; the durable
    // event+recipient claim covers daemon-restart idempotency.
    if let Some(de) = outcome.tail {
        let kind = event.kind.as_u16();
        if first_sight {
            // Presence-lease renewals (kind:30315) are too noisy for info.
            let is_lease_renewal = kind == 30315;
            if is_lease_renewal {
                tracing::debug!(kind, id = %&event.id.to_hex()[..8], "first-sight");
            } else {
                tracing::info!(kind, id = %&event.id.to_hex()[..8], "first-sight");
            }
            derive_and_emit_tail_events(state, &de, &hosted, now);
            if event.kind.as_u16() == crate::fabric::nip29::wire::KIND_CHAT {
                if let DomainEvent::ChatMessage(ref chat) = de {
                    match super::direct_mentions::route(
                        state,
                        super::direct_mentions::DirectMention {
                            event_id: &event.id.to_hex(),
                            from_pubkey: &event.pubkey.to_hex(),
                            channel_h: &chat.channel,
                            body: &chat.body,
                            created_at: event.created_at.as_secs(),
                            target_pubkeys: &chat.mentioned_pubkeys,
                            attachments: &chat.attachments,
                        },
                    ) {
                        Ok(report) if !report.owned_targets.is_empty() => {
                            let st = state.clone();
                            let ev = event.clone();
                            tokio::spawn(async move {
                                route_reaction::publish_eye_reaction(&st, &ev).await;
                            });
                        }
                        Ok(_) => {}
                        Err(error) => tracing::error!(
                            event_id = %event.id,
                            %error,
                            "direct mention routing failed; relay event remains cached"
                        ),
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
        DomainEvent::Status(s) => {
            // Skip own status — local turn/status is tracked by Turn RPC events.
            if hosted.contains(&s.agent.pubkey) {
                return;
            }
            for channel in &s.channels {
                // The unified Status is the sole presence lease, so
                // first-sight of a (pubkey, channel) here is the peer
                // "joined" signal for that channel.
                let key = (s.agent.pubkey.clone(), channel.clone());
                let is_new = {
                    let mut map = state.dedup.peer_sessions.lock().unwrap();
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

                let cur = (s.title.clone(), s.state);
                let should_emit = {
                    let mut map = state.dedup.last_status.lock().unwrap();
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
                        state: s.state,
                    });
                }
            }
        }
        DomainEvent::Profile(pf) => {
            let is_new = {
                let mut set = state.dedup.profiles.lock().unwrap();
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
        DomainEvent::Reaction(_) => {
            // Reactions never reach the tail (materialize() sets tail=None), and
            // even if one did it is passive awareness with no real-time surface.
        }
    }
}

#[cfg(test)]
#[path = "demux/tests.rs"]
mod tests;
