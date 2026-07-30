use super::{wait_until, Home};
use std::time::Duration;

pub(crate) fn daemon_log_boundary(home: &Home) -> usize {
    std::fs::read_to_string(home.dir.path().join("daemon.log"))
        .unwrap_or_default()
        .len()
}

/// Wait until the new daemon has explicitly re-adopted this exact generation.
///
/// Socket readiness is earlier than asynchronous session reconciliation, and
/// the session row is durable across restart. Those facts alone cannot prove
/// the new daemon rebuilt the runtime engine.
pub(crate) fn wait_for_reconciled_session_engine(
    home: &Home,
    pubkey: &str,
    runtime_generation: u64,
    log_boundary: usize,
) {
    let generation = format!("runtime_generation={runtime_generation}");
    let log_path = home.dir.path().join("daemon.log");
    let mut last_tail = String::new();
    assert!(
        wait_until(Duration::from_secs(25), || {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            let tail = log.get(log_boundary..).unwrap_or(&log);
            last_tail = tail.to_string();
            let revived = tail.lines().any(|line| {
                line.contains("reviving session from previous daemon instance")
                    && line.contains(pubkey)
                    && line.contains(&generation)
            });
            let spawned = tail.lines().any(|line| {
                line.contains("spawning session engine")
                    && line.contains(pubkey)
                    && line.contains(&generation)
            });
            revived && spawned
        }),
        "daemon did not re-adopt session {pubkey} generation {runtime_generation}; \
         post-restart daemon log:\n{last_tail}"
    );
}
