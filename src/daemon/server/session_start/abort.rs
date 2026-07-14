use super::super::{session_signing::MintedSession, *};

pub(super) struct SessionStartGuard {
    state: Arc<DaemonState>,
    session_id: String,
    armed: bool,
}

impl SessionStartGuard {
    pub(super) fn new(state: &Arc<DaemonState>, minted: &MintedSession) -> Self {
        Self {
            state: state.clone(),
            session_id: minted.identity.session_id.clone(),
            armed: !minted.identity.durable_agent || minted.durable_claim_acquired,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SessionStartGuard {
    fn drop(&mut self) {
        if self.armed {
            abort_session_start(&self.state, &self.session_id);
        }
    }
}

#[cfg(test)]
#[path = "abort/tests.rs"]
mod tests;

/// Roll back a half-started session before `rpc_session_start` returns an error.
pub(super) fn abort_session_start(state: &Arc<DaemonState>, session_id: &str) {
    if let Err(e) = state.with_store(|s| s.release_durable_agent_session(session_id)) {
        tracing::error!(
            session = %session_id,
            error = %e,
            "failed to release durable-agent binding while aborting session start"
        );
    }
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
    if let Err(e) = state.with_store(|s| s.mark_handle_offline_for_session(session_id)) {
        tracing::error!(session = %session_id, error = %e, "failed to release handle while aborting session start");
    }
    if let Err(e) = state.with_store(|s| s.clear_session_aliases(session_id)) {
        tracing::error!(session = %session_id, error = %e, "failed to clear aliases while aborting session start");
    }
}
