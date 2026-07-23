use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    NotPty,
    ReviveUnavailable,
    RetainUnavailable,
    Gone,
}

/// Classify an unreachable PTY without changing or terminating its process.
/// Exact owned metadata may prove that the supervisor still runs, but only the
/// supervisor control protocol can prove attachment state.
pub(super) fn reconcile(state: &Arc<DaemonState>, session: &crate::state::Session) -> Decision {
    if session.admitted_transport != crate::session_host::transport::TransportKind::Pty.as_str() {
        return Decision::NotPty;
    }
    let endpoint_id = runtime_endpoint_id(state, session);
    let decision = match endpoint_id.as_deref() {
        None => decision_for_observation(None),
        Some(endpoint_id) => match crate::pty::owned_supervisor_state(endpoint_id) {
            Ok(owned) => decision_for_observation(Some(owned)),
            Err(error) => {
                tracing::error!(
                    pubkey = %session.pubkey,
                    runtime_generation = session.runtime_generation,
                    endpoint_id,
                    %error,
                    "PTY ownership could not be proven; retaining unavailable"
                );
                Decision::RetainUnavailable
            }
        },
    };
    if matches!(
        decision,
        Decision::ReviveUnavailable | Decision::RetainUnavailable
    ) {
        let _ = super::super::session_termination::record_control_unavailable(
            state,
            session,
            now_secs(),
        );
    }
    match (decision, endpoint_id.as_deref()) {
        (Decision::ReviveUnavailable, Some(endpoint_id)) => tracing::warn!(
            pubkey = %session.pubkey,
            runtime_generation = session.runtime_generation,
            endpoint_id,
            "owned PTY control is unavailable; reviving without termination"
        ),
        (Decision::RetainUnavailable, endpoint_id) => tracing::error!(
            pubkey = %session.pubkey,
            runtime_generation = session.runtime_generation,
            endpoint_id,
            "unreachable PTY ownership is uncertain; retaining without termination"
        ),
        _ => {}
    }
    decision
}

fn decision_for_observation(owned: Option<crate::pty::OwnedSupervisorState>) -> Decision {
    match owned {
        Some(crate::pty::OwnedSupervisorState::Running) => Decision::ReviveUnavailable,
        Some(crate::pty::OwnedSupervisorState::Gone) => Decision::Gone,
        Some(crate::pty::OwnedSupervisorState::Missing) | None => Decision::RetainUnavailable,
    }
}

fn runtime_endpoint_id(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
) -> Option<String> {
    state
        .with_store(|store| {
            store.runtime_locator_for_session(
                &session.pubkey,
                session.runtime_generation,
                crate::state::LOCATOR_PTY,
            )
        })
        .ok()?
        .map(|locator| locator.locator_value)
}

#[cfg(test)]
#[path = "unreachable_pty/tests.rs"]
mod tests;
