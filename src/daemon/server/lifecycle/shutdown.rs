use super::*;

pub(super) async fn rpc_sessions(state: &Arc<DaemonState>) {
    super::super::session_termination::shutdown_daemon_owned_rpc_sessions(state).await;
}
