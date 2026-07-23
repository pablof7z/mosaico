use super::*;

#[derive(serde::Deserialize)]
struct PresentationChangedParams {
    pty_id: String,
    presentation: crate::pty::PresentationSnapshot,
}

pub(super) fn rpc_changed(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: PresentationChangedParams = serde_json::from_value(params.clone())
        .context("parsing pty_presentation_changed params")?;
    let Some(session) = super::session_for_pty(state, &params.pty_id)? else {
        return Ok(serde_json::json!({"applied": false}));
    };
    let applied = apply(state, &session, params.presentation)?;
    Ok(serde_json::json!({"applied": applied}))
}

pub(super) async fn reconcile(state: &Arc<DaemonState>) {
    let sessions = state.with_store(|store| store.list_running_sessions().unwrap_or_default());
    for session in sessions {
        let locators = state
            .with_store(|store| store.locators_for_pubkey(&session.pubkey))
            .unwrap_or_default();
        if let Some(pty) = locators.iter().find(|locator| {
            locator.locator_kind == crate::state::LOCATOR_PTY
                && locator.runtime_generation == session.runtime_generation
        }) {
            reconcile_pty(state, &session, &pty.locator_value).await;
        } else if has_live_rpc_locator(&locators, session.runtime_generation)
            && session.presentation_state != PresentationState::Headless
        {
            let _ = state.with_store(|store| {
                store.apply_session_presentation_edge(
                    &session.pubkey,
                    session.runtime_generation,
                    session.attachment_epoch.saturating_add(1),
                    PresentationState::Headless,
                    now_secs(),
                )
            });
        }
    }
}

fn has_live_rpc_locator(locators: &[crate::state::SessionLocator], generation: u64) -> bool {
    locators.iter().any(|locator| {
        matches!(
            locator.locator_kind.as_str(),
            crate::state::LOCATOR_ACP | crate::state::LOCATOR_APP_SERVER
        ) && locator.runtime_generation == generation
    })
}

async fn reconcile_pty(state: &Arc<DaemonState>, session: &Session, pty_id: &str) {
    match crate::pty::presentation_observation(pty_id) {
        Ok(observation) => {
            state
                .runtime
                .pty_probe_failures
                .lock()
                .unwrap()
                .remove(&(session.pubkey.clone(), session.runtime_generation));
            if let Err(error) = apply(state, session, observation) {
                tracing::warn!(pubkey = %session.pubkey, %error, "PTY presentation reconciliation failed");
            }
        }
        Err(error)
            if session
                .child_pid
                .is_some_and(super::super::engine_lifecycle::pid_alive) =>
        {
            handle_live_probe_failure(state, session, pty_id, error).await;
        }
        Err(error) => {
            tracing::warn!(pubkey = %session.pubkey, %error, "PTY supervisor is gone");
            let _ = super::stop_generation(state, session, StopReason::Crash, now_secs()).await;
        }
    }
}

async fn handle_live_probe_failure(
    state: &Arc<DaemonState>,
    session: &Session,
    _pty_id: &str,
    error: crate::pty::PresentationUnavailable,
) {
    let failures = {
        let mut probes = state.runtime.pty_probe_failures.lock().unwrap();
        let count = probes
            .entry((session.pubkey.clone(), session.runtime_generation))
            .or_insert(0);
        *count = count.saturating_add(1);
        *count
    };
    if let Err(persist_error) =
        super::super::session_termination::record_control_unavailable(state, session, now_secs())
    {
        tracing::error!(
            pubkey = %session.pubkey,
            failures,
            %persist_error,
            "PTY presentation loss could not be persisted"
        );
    }
    match failures {
        1 | 2 => tracing::warn!(
            pubkey = %session.pubkey,
            failures,
            %error,
            "PTY presentation probe failed; runtime retained unavailable"
        ),
        3 => tracing::error!(
            pubkey = %session.pubkey,
            failures,
            %error,
            "PTY presentation remains unavailable; automatic termination denied"
        ),
        _ => tracing::debug!(
            pubkey = %session.pubkey,
            failures,
            %error,
            "PTY presentation still unavailable"
        ),
    }
}

pub(super) fn apply(
    state: &Arc<DaemonState>,
    session: &Session,
    snapshot: crate::pty::PresentationSnapshot,
) -> Result<bool> {
    let presentation = if snapshot.is_headless() {
        PresentationState::Headless
    } else {
        PresentationState::Headed
    };
    state.with_store(|store| match session.runtime_state {
        RuntimeState::Running => store.apply_session_presentation_edge(
            &session.pubkey,
            session.runtime_generation,
            snapshot.attachment_epoch,
            presentation,
            snapshot.changed_at,
        ),
        RuntimeState::Stopping => store.cancel_idle_eviction_on_presentation_change(
            &session.pubkey,
            session.runtime_generation,
            session.lifecycle_epoch,
            snapshot.attachment_epoch,
            presentation,
            snapshot.changed_at,
        ),
        RuntimeState::Stopped => Ok(false),
    })
}

#[cfg(test)]
#[path = "presentation/tests.rs"]
mod tests;
