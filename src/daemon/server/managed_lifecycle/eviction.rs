use super::*;
use crate::session_host::transport::{HostedEndpoint, TransportKind};

pub(super) async fn evict_due_idle_sessions(state: &Arc<DaemonState>) {
    let candidates = state
        .with_store(|store| store.list_due_idle_evictions(now_secs()))
        .unwrap_or_default();
    for candidate in candidates {
        if let Err(error) = evict_one(state, candidate).await {
            tracing::warn!(%error, "idle session eviction failed safely");
        }
    }
}

pub(super) async fn reconcile_stopping(state: &Arc<DaemonState>) {
    let sessions = state
        .with_store(|store| store.list_stopping_sessions())
        .unwrap_or_default();
    for session in sessions {
        if session.stop_reason != Some(StopReason::IdleEvicted) {
            tracing::error!(
                pubkey = %session.pubkey,
                reason = ?session.stop_reason,
                "recovering stopping session with invalid ownership marker"
            );
        }
        if let Err(error) = finish_idle_eviction(state, session).await {
            tracing::warn!(%error, "stopping session reconciliation remains retryable");
        }
    }
}

async fn evict_one(state: &Arc<DaemonState>, candidate: Session) -> Result<()> {
    let Some(stopping) = state.with_store(|store| {
        store.reserve_due_idle_eviction(
            &candidate.pubkey,
            candidate.runtime_generation,
            candidate.lifecycle_epoch,
            candidate.attachment_epoch,
            now_secs(),
        )
    })?
    else {
        return Ok(());
    };
    finish_idle_eviction(state, stopping).await
}

async fn finish_idle_eviction(state: &Arc<DaemonState>, stopping: Session) -> Result<()> {
    let endpoint = state.with_store(|store| {
        crate::session_host::transport::hosted_endpoint_for(store, &stopping)
    })?;
    let locator_kind = match &endpoint {
        HostedEndpoint::Resolved { endpoint, .. } => Some(endpoint.kind.locator_kind()),
        HostedEndpoint::Unavailable { kind } => Some(kind.locator_kind()),
        HostedEndpoint::Unhosted => None,
    };
    match &endpoint {
        HostedEndpoint::Resolved {
            endpoint,
            transport: _,
        } if endpoint.kind == TransportKind::Pty => {
            let id = endpoint.endpoint_id.as_str();
            match crate::pty::kill_if_headless_at(id, stopping.attachment_epoch) {
                Ok(crate::pty::ConditionalKillOutcome::Killed { .. }) => {}
                Ok(crate::pty::ConditionalKillOutcome::PresentationChanged { presentation }) => {
                    if !super::presentation::apply(state, &stopping, presentation)? {
                        ensure_not_stuck_stopping(state, &stopping)?;
                    }
                    return Ok(());
                }
                Err(error) => {
                    if crate::pty::is_live(id) {
                        let cancelled = state.with_store(|store| {
                            store.cancel_idle_eviction_on_presentation_change(
                                &stopping.pubkey,
                                stopping.runtime_generation,
                                stopping.lifecycle_epoch,
                                stopping.attachment_epoch,
                                PresentationState::Unavailable,
                                now_secs(),
                            )
                        })?;
                        if !cancelled {
                            ensure_not_stuck_stopping(state, &stopping)?;
                        }
                        return Err(error.into());
                    }
                    // A closed socket is not proof that the runtime is gone. Use
                    // the persisted supervisor identity, including its instance
                    // token, to confirm exit or terminate it before committing
                    // the durable stopped edge. Missing metadata is safe only
                    // when the recorded supervisor PID is already absent.
                    match crate::pty::terminate_owned_supervisor(id) {
                        Ok(true) => {}
                        Ok(false)
                            if stopping
                                .child_pid
                                .is_some_and(super::super::engine_lifecycle::pid_alive) =>
                        {
                            return Err(error.into());
                        }
                        Ok(false) => {}
                        Err(ownership_error) => return Err(ownership_error),
                    }
                }
            }
        }
        HostedEndpoint::Resolved {
            transport,
            endpoint,
        } => {
            transport.kill(endpoint).await?;
        }
        HostedEndpoint::Unavailable { kind } => {
            if stopping
                .child_pid
                .is_some_and(super::super::engine_lifecycle::pid_alive)
            {
                anyhow::bail!(
                    "refusing to stop {} runtime without its owned endpoint",
                    kind.as_str()
                );
            }
        }
        HostedEndpoint::Unhosted => {
            if stopping
                .child_pid
                .is_some_and(super::super::engine_lifecycle::pid_alive)
            {
                anyhow::bail!(
                    "refusing to signal an unbound live process without an owned runtime endpoint"
                );
            }
        }
    }
    let stopped_at = now_secs();
    cancel_session(state, &stopping.pubkey, stopping.runtime_generation);
    let stopped = state.with_store(|store| {
        store.finalize_runtime_stopped_if_epoch(
            &stopping.pubkey,
            stopping.runtime_generation,
            stopping.lifecycle_epoch,
            StopReason::IdleEvicted,
            stopped_at,
        )
    })?;
    if let Some(stopped) = stopped {
        state.with_store(|store| {
            store.clear_runtime_locator_if_generation(
                &stopped.pubkey,
                locator_kind.unwrap_or(crate::state::LOCATOR_PID),
                stopped.runtime_generation,
            )
        })?;
        super::super::presence::close_generation(
            state,
            &stopped.pubkey,
            stopped.runtime_generation,
            stopped_at,
            "idle_eviction_stopped",
        )
        .await;
        super::emit_stopped(state, &stopped, stopped_at);
    }
    Ok(())
}

fn ensure_not_stuck_stopping(state: &Arc<DaemonState>, expected: &Session) -> Result<()> {
    let current = state.with_store(|store| store.get_session(&expected.pubkey))?;
    if current.is_some_and(|session| {
        session.runtime_generation == expected.runtime_generation
            && session.lifecycle_epoch == expected.lifecycle_epoch
            && session.runtime_state == RuntimeState::Stopping
    }) {
        anyhow::bail!("stopping lifecycle edge was not cancelled")
    }
    Ok(())
}
