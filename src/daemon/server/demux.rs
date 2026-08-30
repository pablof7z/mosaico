//! Relay demux pipeline extracted from `server.rs` (issue #12, EPIC-server-001).
//!
//! One relay subscription feeds every hosted agent. `spawn_demux` drains the
//! notification stream; incoming Fava rows are decoded once and derive
//! real-time `TailEvent`s via `derive_and_emit_tail_events`. The two
//! async side-channels (`handle_offline_agent_mention`, `handle_orchestration`)
//! are dispatched off the demux loop.
//!
//! `spawn_demux` and `handle_orchestration` are `pub(super)` because the parent
//! module calls them (the accept-loop bootstrap and the channel_create local
//! fast-path); everything else is private to this module.

use super::*;

mod arrival;
mod attachments;
mod chat_ops;
mod inbound_dispatch;
mod offline_mention;
mod route_reaction;
mod tail_projection;

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

pub(super) fn spawn_demux(state: Arc<DaemonState>) {
    let mut transitions = state
        .fava()
        .take_view_transitions()
        .expect("Fava view transition stream has one daemon owner");
    tokio::spawn(async move {
        while let Some(transition) = transitions.recv().await {
            apply_transition(&state, transition).await;
        }
    });
}

/// Apply one Fava frame's row transition.
///
/// **Removals first, additions second, always.** The frame is a set, not a
/// sequence: Fava re-folds it through a `BTreeMap<EventId, _>` before delivery,
/// so the order deltas arrive in is event-id ascending and says nothing about
/// causality. A relay republishing an addressable event — which every NIP-29
/// roster change is — sends `Removed(old)` and `Added(new)` in that one frame,
/// and the view transition captures the departed Row before deindexing it.
/// Acting on each Fava delta as it arrives can therefore blank a roster on
/// nothing but hex ordering.
///
/// Removals-first is also what Fava's own coordinator does with the same
/// problem (`fava_nip65::observe_current_delta`: *"Removals are applied before
/// additions irrespective of delivery order. This lets a replaceable winner's
/// removal reveal an older current row in the same batch without emitting a
/// transient absence."*).
async fn apply_transition(state: &Arc<DaemonState>, transition: crate::fava_views::RowTransition) {
    for departed in transition.removed {
        apply_departure(state, departed);
    }
    let mut hosted = None;
    for entered in transition.entered {
        apply_observation_entry(state, entered, &mut hosted);
    }
    for row in transition.added {
        let event = row.event;
        let event_id = event.id.to_hex();
        arrival::record_before_dispatch(state, &event_id).await;
        inbound_dispatch::dispatch(state, &event);
    }
}

/// Turn one broad-`#h` status entry into peer tail state, once per channel the
/// status names. Under the single broad `#h` observation a kind:30315 event
/// enters exactly once (not once per channel-observation), so the channels it
/// applies to are read from the event's own `#h` tags (`status.channels`)
/// rather than from the observation id.
fn apply_observation_entry(
    state: &Arc<DaemonState>,
    entered: crate::fava_views::EnteredRow,
    hosted: &mut Option<Vec<String>>,
) {
    if entered.observation_id != crate::reconcile::subscriptions::BROAD_H_ID {
        return;
    }
    let event = entered.row.event;
    let Some(DomainEvent::Status(status)) = state
        .provider()
        .decode(&crate::fabric::RawEnvelope::Nostr(event))
    else {
        return;
    };
    let hosted = hosted.get_or_insert_with(|| locally_hosted_pubkeys(state));
    for channel in &status.channels {
        tail_projection::derive_and_emit_status_tail_event(
            state,
            &status,
            channel,
            hosted,
            now_secs(),
        );
    }
}

fn locally_hosted_pubkeys(state: &Arc<DaemonState>) -> Vec<String> {
    let mut hosted = state.hosted_pubkeys();
    hosted.extend(crate::identity::list_local_pubkeys(
        &crate::config::mosaico_home(),
    ));
    hosted.extend(state.with_store(|store| store.list_local_session_pubkeys().unwrap_or_default()));
    hosted.sort_unstable();
    hosted.dedup();
    hosted
}

/// Turn a broad-`#h` status departure into an immediate peer leave, once per
/// channel the departed status named that is no longer current. A replacement
/// delivered in the same frame is already installed in the Fava view, so
/// `get_status` suppresses a transient Leave for channels the agent still
/// occupies without introducing a Mosaico freshness or expiry clock.
fn apply_departure(state: &Arc<DaemonState>, departed: crate::fava_views::DepartedRow) {
    if departed.observation_id != crate::reconcile::subscriptions::BROAD_H_ID {
        return;
    }
    let event = departed.row.event;
    let Some(DomainEvent::Status(status)) = state
        .provider()
        .decode(&crate::fabric::RawEnvelope::Nostr(event))
    else {
        return;
    };
    let now = now_secs();
    for channel in &status.channels {
        let key = (status.agent.pubkey.clone(), channel.clone());
        let still_current = state.with_store(|store| {
            store
                .get_status(&key.0, &key.1)
                .is_ok_and(|status| status.is_some())
        });
        if still_current {
            continue;
        }
        let tracked = state.dedup.peer_sessions.lock().unwrap().remove(&key);
        state.dedup.last_status.lock().unwrap().remove(&key);
        let Some(tracked) = tracked else {
            continue;
        };
        state.emit_tail(TailEvent::Leave {
            ts: now,
            channel: tracked.channel,
            agent: tracked.slug,
            host: tracked.host,
            session: key.0,
            online_s: now.saturating_sub(tracked.first_seen),
        });
    }
}

fn finish_incoming(
    state: &Arc<DaemonState>,
    event: &Event,
    decoded: crate::fabric::ProductDecode,
    now: u64,
) {
    // Each event reaches here exactly once. The FavaViews `transition.added`
    // set only carries rows newly added to the views, and under the single
    // broad `#h` observation (mosaico#837) there is no overlapping delivery to
    // re-add a row, so the per-process `first_sight` LRU that guarded this
    // broadcast is retired. Durable idempotency across daemon restarts lives
    // in `event_claims` + the `inbox` ledger.
    if let Some(de) = decoded.tail {
        let kind = event.kind.as_u16();
        // Presence-lease renewals (kind:30315) are too noisy for info.
        if kind == 30315 {
            tracing::debug!(kind, id = %&event.id.to_hex()[..8], "incoming");
        } else {
            tracing::info!(kind, id = %&event.id.to_hex()[..8], "incoming");
        }
        tail_projection::derive_and_emit_tail_events(state, &de, now);
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
                        "direct mention routing failed; Fava row remains observed"
                    ),
                }
            }
        }
    }
    chat_ops::dispatch(state, event);
}

#[cfg(test)]
#[path = "demux/tests.rs"]
mod tests;
