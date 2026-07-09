//! Peer-session pruner: emits `Leave` tail events for peer presences that have
//! expired out of the store. Extracted from `server.rs` (issue #12).

use super::super::*;

/// Every 30s, drop peer sessions older than `PRUNE_PEER_AFTER_SECS` from the
/// store and emit a `Leave` tail event for each `(pubkey, session, channel)`
/// tuple that was tracked in memory but is no longer live.
pub fn spawn_pruner(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let now = now_secs();

            // Reap locally-managed sessions that crashed or went silent past the
            // membership TTL. Boot and session-start also sweep, but a session that
            // dies mid-run with no further lifecycle event would otherwise linger on
            // its channel rosters until the daemon restarts — this timer closes that
            // gap (~10 min worst case).
            super::super::membership_cleanup::cleanup_dead_local_sessions(&state);

            match state.with_store(|s| s.prune_retained_state(now)) {
                Ok(report) if report.total() > 0 => tracing::debug!(
                    relay_events = report.relay_events,
                    delivered_inbox = report.delivered_inbox,
                    published_outbox = report.published_outbox,
                    "pruned retained state"
                ),
                Ok(_) => {}
                Err(e) => tracing::error!(
                    error = %format!("{e:#}"),
                    "state retention prune failed"
                ),
            }

            // Identify which peer sessions will be pruned by checking the map
            // against sessions that are about to expire.
            let tracked_keys: Vec<(String, String, String)> = {
                let map = state.peer_sessions.lock().unwrap();
                // We'll emit Leave for (pubkey, session, channel) tuples in our map
                // that are no longer live in the store. Cross-reference below.
                map.keys().cloned().collect()
            };

            // Which tracked tuples are still live. Peer presence is now
            // read from the relay_status cache (NIP-40 liveness), not a dedicated
            // peer-sessions table: a tuple is alive while its kind:30315 has not
            // expired. The cache is relay-materialized, so there is nothing to
            // manually prune — expired rows simply read as not-live.
            let still_alive: std::collections::HashSet<(String, String, String)> = state
                .with_store(|s| {
                    tracked_keys
                        .iter()
                        .filter(|(pubkey, session_id, channel)| {
                            s.get_status(pubkey, session_id, channel)
                                .ok()
                                .flatten()
                                .map(|st| st.expiration >= now)
                                .unwrap_or(false)
                        })
                        .cloned()
                        .collect()
                });

            // Emit Leave for tuples that were in our map but are now expired.
            let to_leave: Vec<((String, String), PeerTracked)> = {
                let mut map = state.peer_sessions.lock().unwrap();
                let expired: Vec<(String, String, String)> = tracked_keys
                    .into_iter()
                    .filter(|k| !still_alive.contains(k))
                    .collect();
                let mut leaves = Vec::new();
                for key in expired {
                    if let Some(tracked) = map.remove(&key) {
                        leaves.push(((key.0, key.1), tracked));
                    }
                }
                leaves
            };
            for ((_pubkey, session_id), tracked) in to_leave {
                let online_s = now.saturating_sub(tracked.first_seen);
                state.emit_tail(TailEvent::Leave {
                    ts: now,
                    channel: tracked.channel,
                    agent: tracked.slug,
                    host: tracked.host,
                    session: session_id,
                    online_s,
                });
            }
        }
    });
}
