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
    let mut batches = state
        .nmp()
        .take_materialization_events()
        .expect("NMP materialization stream has one daemon owner");
    tokio::spawn(async move {
        while let Some(batch) = batches.recv().await {
            if !state.nmp().accepts_materialization(&batch) {
                continue;
            }
            apply_batch(&state, batch);
        }
    });
}

/// Apply one NMP frame's row transition.
///
/// **Removals first, additions second, always.** The frame is a set, not a
/// sequence: NMP re-folds it through a `BTreeMap<EventId, _>` before delivery,
/// so the order deltas arrive in is event-id ascending and says nothing about
/// causality. A relay republishing an addressable event — which every NIP-29
/// roster change is — sends `Removed(old)` and `Added(new)` in that one frame,
/// and `Removed` carries only an id. Acting on each delta as it arrives can
/// therefore blank a roster on nothing but hex ordering.
///
/// Removals-first is also what NMP's own coordinator does with the same
/// problem (`nmp_nip65::observe_current_delta`: *"Removals are applied before
/// additions irrespective of delivery order. This lets a replaceable winner's
/// removal reveal an older current row in the same batch without emitting a
/// transient absence."*).
fn apply_batch(state: &Arc<DaemonState>, batch: crate::nmp_host::MaterializationBatch) {
    if batch.phase == crate::nmp_host::MaterializationPhase::Closed {
        let orphaned =
            state.with_store(|store| store.close_projection_observation(&batch.observation_id));
        retract_orphans(state, orphaned, &batch.observation_id);
        return;
    }

    let evidence_json = match batch.evidence_json() {
        Ok(evidence) => evidence,
        Err(error) => {
            tracing::error!(
                observation = %batch.observation_id,
                %error,
                "NMP frame evidence serialization failed; projection frame refused"
            );
            return;
        }
    };
    let relay_settled = batch.relay_settled();
    let accepted = state.with_store(|store| {
        store.begin_projection_frame(
            &batch.observation_id,
            batch.generation,
            &evidence_json,
            relay_settled,
        )
    });
    match accepted {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::error!(
                observation = %batch.observation_id,
                %error,
                "recording NMP frame evidence failed; projection frame refused"
            );
            return;
        }
    }

    for id in &batch.removed {
        let event_id = id.to_hex();
        match state
            .with_store(|store| store.release_projection_event(&batch.observation_id, &event_id))
        {
            Ok(true) => retract_orphans(state, Ok(vec![event_id]), &batch.observation_id),
            Ok(false) => {}
            Err(error) => tracing::error!(
                observation = %batch.observation_id,
                event_id,
                %error,
                "releasing NMP projection owner failed"
            ),
        }
    }
    for growth in &batch.sources_grew {
        let sources_json = match growth.sources_json() {
            Ok(sources) => sources,
            Err(error) => {
                tracing::error!(event_id = %growth.id, %error, "serializing relay sources failed");
                continue;
            }
        };
        if let Err(error) = state.with_store(|store| {
            store.grow_projection_event_sources(
                &batch.observation_id,
                &growth.id.to_hex(),
                &sources_json,
            )
        }) {
            tracing::error!(event_id = %growth.id, %error, "recording relay source growth failed");
        }
    }
    for row in &batch.added {
        let sources_json = match row.sources_json() {
            Ok(sources) => sources,
            Err(error) => {
                tracing::error!(event_id = %row.event.id, %error, "serializing relay sources failed");
                continue;
            }
        };
        let event_id = row.event.id.to_hex();
        if let Err(error) = state.with_store(|store| {
            store.claim_projection_event(
                &batch.observation_id,
                batch.generation,
                &event_id,
                &sources_json,
            )
        }) {
            tracing::error!(event_id, %error, "recording NMP projection owner failed");
            continue;
        }
        inbound_dispatch::dispatch(
            state,
            &row.event,
            crate::fabric::ProjectionProvenance {
                source_event_id: event_id,
            },
        );
    }

    if relay_settled {
        let orphaned = state.with_store(|store| {
            store.settle_projection_frame(&batch.observation_id, batch.generation)
        });
        retract_orphans(state, orphaned, &batch.observation_id);
    }
}

fn retract_orphans(
    state: &Arc<DaemonState>,
    orphaned: anyhow::Result<Vec<String>>,
    observation_id: &str,
) {
    let orphaned = match orphaned {
        Ok(orphaned) => orphaned,
        Err(error) => {
            tracing::error!(observation = observation_id, %error, "releasing projection owners failed");
            return;
        }
    };
    state.with_store(|store| {
        for id in orphaned {
            match store.retract_projection_source(&id) {
                Ok(true) => tracing::debug!(id = %&id[..8], "retracted — NMP dropped the row"),
                Ok(false) => {}
                Err(error) => tracing::error!(
                    id = %&id[..8],
                    %error,
                    "retraction failed; a deleted or expired event stays cached"
                ),
            }
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
