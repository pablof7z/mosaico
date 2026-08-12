//! Periodically prune local completed-work ledgers.

use super::super::*;

/// Every 30s, prune completed local ledgers and drive offline-mention retries.
/// Status disappearance arrives directly through NMP Row transitions.
pub fn spawn_pruner(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let now = now_secs();

            super::super::demux::drive_offline_mention_retries(&state);

            match state.with_store(|s| s.prune_retained_state(now)) {
                Ok(report) if report.total() > 0 => tracing::debug!(
                    delivered_inbox = report.delivered_inbox,
                    completed_event_claims = report.completed_event_claims,
                    native_turn_attempts = report.native_turn_attempts,
                    "pruned retained state"
                ),
                Ok(_) => {}
                Err(e) => tracing::error!(
                    error = %format!("{e:#}"),
                    "state retention prune failed"
                ),
            }
        }
    });
}
