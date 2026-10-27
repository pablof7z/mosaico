//! Relay-to-product dispatch from the complete decoded NIP-29 view.
//!
//! Subscription notifications are wake-only. Every wake rereads the complete
//! current level; the Nostr runtime remains the sole owner of raw relay truth. Durable
//! arrival, inbox, and event-claim rows make local effects replay-safe.

use super::*;
use std::collections::BTreeSet;

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
    let mut changes = state.nip29_view().subscribe_changes();
    tokio::spawn(async move {
        loop {
            apply_current(&state).await;
            if changes.changed().await.is_err() {
                return;
            }
        }
    });
}

async fn apply_current(state: &Arc<DaemonState>) {
    let view = state.nip29_view().current();
    let hosted = locally_hosted_pubkeys(state);
    let mut current_statuses = BTreeSet::new();

    for event in view.content() {
        apply_current_status(state, event, &hosted, &mut current_statuses);
        let first_arrival = arrival::record_before_dispatch(state, &event.id.to_hex()).await;
        inbound_dispatch::dispatch(state, &event, first_arrival);
    }

    apply_status_departures(state, &current_statuses);
}

fn apply_current_status(
    state: &Arc<DaemonState>,
    event: &Event,
    hosted: &[String],
    current: &mut BTreeSet<(String, String)>,
) {
    let Some(DomainEvent::Status(status)) = state
        .provider()
        .decode(&crate::fabric::RawEnvelope::Nostr(event.clone()))
    else {
        return;
    };
    for channel in &status.channels {
        current.insert((status.agent.pubkey.clone(), channel.clone()));
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

fn apply_status_departures(state: &Arc<DaemonState>, current: &BTreeSet<(String, String)>) {
    let departed = state
        .dedup
        .peer_sessions
        .lock()
        .unwrap()
        .keys()
        .filter(|key| !current.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    let now = now_secs();
    for key in departed {
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
    first_arrival: bool,
) {
    if let Some(de) = decoded.tail {
        if first_arrival {
            let kind = event.kind.as_u16();
            if kind == 30315 {
                tracing::debug!(kind, id = %&event.id.to_hex()[..8], "incoming");
            } else {
                tracing::info!(kind, id = %&event.id.to_hex()[..8], "incoming");
            }
            tail_projection::derive_and_emit_tail_events(state, &de, now);
        }
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
                    Ok(report) if report.newly_parked && !report.owned_targets.is_empty() => {
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
                        "direct mention routing failed; Nostr runtime event remains current"
                    ),
                }
            }
        }
    }
    chat_ops::dispatch(state, event);
}
