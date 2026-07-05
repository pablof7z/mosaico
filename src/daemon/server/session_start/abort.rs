use super::super::*;

/// Roll back a half-started session before `rpc_session_start` returns an error.
pub(super) fn abort_session_start(state: &Arc<DaemonState>, session_id: &str) {
    state.release_session_signer(session_id);
    if let Err(e) = state.with_store(|s| s.mark_dead(session_id)) {
        tracing::error!(
            session = %session_id,
            error = %e,
            "failed to mark session row dead while aborting session start (ghost-alive row may remain)"
        );
    }
    if let Err(e) = state.with_store(|s| s.mark_identity_dead_for_session(session_id)) {
        tracing::error!(
            session = %session_id,
            error = %e,
            "failed to mark identity dead while aborting session start"
        );
    }
}
