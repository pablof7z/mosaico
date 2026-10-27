use super::*;

pub(super) async fn rpc_sessions(state: &Arc<DaemonState>) {
    super::super::session_termination::shutdown_daemon_owned_rpc_sessions(state).await;
}

pub(super) async fn wait_for_termination(state: &Arc<DaemonState>) {
    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    tokio::select! {
        _ = state.connections.shutdown.notified() => {}
        _ = async {
            match &mut sigterm {
                Some(signal) => { signal.recv().await; }
                None => std::future::pending().await,
            }
        } => {}
    }
}

/// Tests and labs may explicitly request that daemon-owned PTY supervisors end.
pub(super) fn pty_supervisors_if_requested() {
    if !crate::pty::reap_sessions_on_stop_enabled() {
        return;
    }
    match crate::pty::reap_home_supervisors() {
        Ok(report) => {
            if !report.reaped.is_empty() {
                tracing::info!(
                    count = report.reaped.len(),
                    "reaped PTY supervisors on daemon stop"
                );
            }
            for error in report.errors {
                tracing::warn!(%error, "PTY supervisor reap failed during daemon stop");
            }
        }
        Err(error) => tracing::warn!(%error, "PTY supervisor reap aborted during daemon stop"),
    }
}
