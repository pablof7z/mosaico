use super::*;

pub(super) async fn rpc_sessions(state: &Arc<DaemonState>) {
    super::super::session_termination::shutdown_daemon_owned_rpc_sessions(state).await;
}

/// When tests/labs set `MOSAICO_REAP_SESSIONS_ON_STOP`, also kill every PTY
/// supervisor owned by this `$MOSAICO_HOME`. Production leaves the flag unset so
/// detached sessions survive daemon restart (see AGENTS.md).
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
        Err(error) => {
            tracing::warn!(%error, "PTY supervisor reap aborted during daemon stop");
        }
    }
}
