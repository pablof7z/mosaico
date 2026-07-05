//! Generic outbox drainer. The per-session status **decision** now lives in the
//! Trellis [`crate::reconcile::status`] reconciler (the single change-only publish
//! authority); the runtime engine feeds it inputs and enqueues the emitted
//! publish/expire effects here. The old second heartbeat timer
//! (`spawn_status_heartbeat_publisher`, a direct `set_status` path with no dedup)
//! was DELETED — TTL re-arm is now the reconciler's `on_tick`, published through
//! this one outbox executor.

use super::*;

#[cfg(test)]
mod tests;

/// Drain the generic outbox: publish each queued signed event JSON via the
/// transport with a checked relay verdict, marking it published only after a
/// relay accepts it. Failed attempts stay pending with an error and retry count.
/// Woken instantly by `outbox_notify` on every enqueue, and polled every 2s as a
/// fallback.
pub(in crate::daemon::server) fn spawn_outbox_drainer(state: Arc<DaemonState>) {
    use nostr_sdk::prelude::{Event, JsonUtil};
    tokio::spawn(async move {
        loop {
            // Publish the backlog while we keep making progress (so a startup
            // burst clears fast); stop if a whole batch failed to avoid a tight spin.
            loop {
                let items = state.with_store(|s| s.peek_outbox(32).unwrap_or_default());
                if items.is_empty() {
                    break;
                }
                let mut progressed = false;
                for item in items {
                    match Event::from_json(&item.event_json) {
                        Ok(ev) => match state.transport.publish_event_checked(&ev).await {
                            Ok(_) => {
                                let fact = crate::reconcile::InputFact::RelayPublishAccepted {
                                    local_id: item.local_id,
                                    event_id: ev.id.to_hex(),
                                    accepted: true,
                                    error: None,
                                    at: now_secs(),
                                };
                                if let Err(e) = crate::outbox_seam::drive(
                                    &state.outbox,
                                    &state.store,
                                    "relay_publish",
                                    fact,
                                ) {
                                    tracing::error!(error = %e, "outbox publish ack was not applied");
                                }
                                progressed = true;
                            }
                            Err(e) => {
                                apply_publish_failure(&state, item.local_id, &ev.id.to_hex(), e);
                            }
                        },
                        Err(e) => {
                            // A row we can never parse is dead; record the error.
                            // It stays pending but won't block other rows.
                            apply_publish_failure(
                                &state,
                                item.local_id,
                                "",
                                anyhow::anyhow!("bad event json: {e}"),
                            );
                        }
                    }
                }
                if !progressed {
                    break;
                }
            }
            tokio::select! {
                _ = state.outbox_notify.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
    });
}

fn apply_publish_failure(
    state: &Arc<DaemonState>,
    local_id: i64,
    event_id: &str,
    error: anyhow::Error,
) {
    let message = format!("{error:#}");
    let fact = crate::reconcile::InputFact::RelayPublishAccepted {
        local_id,
        event_id: event_id.to_string(),
        accepted: false,
        error: Some(message),
        at: now_secs(),
    };
    if let Err(e) = crate::outbox_seam::drive(&state.outbox, &state.store, "relay_publish", fact) {
        tracing::error!(error = %e, "outbox publish failure was not applied");
    }
}

// ── session lifecycle ─────────────────────────────────────────────────────────
