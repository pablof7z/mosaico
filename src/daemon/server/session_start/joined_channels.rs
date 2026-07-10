use super::*;

pub(super) fn record(
    state: &Arc<DaemonState>,
    session_id: &str,
    primary: String,
    mut requested: Vec<String>,
    now: u64,
) -> Vec<String> {
    requested.push(primary);
    requested.sort();
    requested.dedup();
    state.with_store(|s| {
        for joined in &requested {
            if let Err(e) = s.join_session_channel(session_id, joined, now) {
                tracing::error!(session = %session_id, channel = %joined, error = %e, "session_start: failed to join requested channel");
            }
        }
    });
    requested
}

pub(super) fn schedule_subscriptions(
    state: &Arc<DaemonState>,
    joined_channels: &[String],
    active_channel: &str,
) {
    for joined in joined_channels.iter().filter(|ch| *ch != active_channel) {
        let st = state.clone();
        let joined = joined.clone();
        tokio::spawn(async move {
            let _ = ensure_subscription(&st, &joined).await;
        });
    }
}
