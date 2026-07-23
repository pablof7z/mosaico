//! Durable managed-runtime lifecycle coordinator.

use super::*;
use crate::state::{PresentationState, RuntimeState, Session, StopReason};

mod eviction;
mod presentation;
mod standing;
pub(super) use standing::commit_confirmed_admission;

pub(super) async fn reconcile_stopping(state: &Arc<DaemonState>) {
    eviction::reconcile_stopping(state).await;
}

const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);

pub(super) fn spawn(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RECONCILE_INTERVAL);
        loop {
            tick.tick().await;
            replay_supervisor_exits(&state).await;
            presentation::reconcile(&state).await;
            standing::reconcile_running(&state).await;
            eviction::reconcile_stopping(&state).await;
            eviction::evict_due_idle_sessions(&state).await;
            standing::reconcile_expired(&state).await;
        }
    });
}

pub(super) async fn replay_supervisor_exits(state: &Arc<DaemonState>) {
    for report in crate::pty::read_exit_reports() {
        match session_for_pty(state, &report.pty_id) {
            Ok(None) if now_secs().saturating_sub(report.recorded_at) < 300 => continue,
            Ok(None) => {
                crate::pty::remove_exit_report(&report.pty_id);
                continue;
            }
            Err(error) => {
                tracing::warn!(pty_id = %report.pty_id, %error, "exit report lookup failed");
                continue;
            }
            Ok(Some(_)) => {}
        }
        match supervisor_exited(
            state,
            &report.pty_id,
            report.child_success,
            report.presentation,
            report.recorded_at,
        )
        .await
        {
            Ok(_) => crate::pty::remove_exit_report(&report.pty_id),
            Err(error) => tracing::warn!(
                pty_id = %report.pty_id,
                %error,
                "persisted supervisor exit remains pending"
            ),
        }
    }
}

pub(super) fn rpc_pty_presentation_changed(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    presentation::rpc_changed(state, params)
}

pub(super) async fn supervisor_exited(
    state: &Arc<DaemonState>,
    pty_id: &str,
    child_success: Option<bool>,
    presentation: crate::pty::PresentationSnapshot,
    exited_at: u64,
) -> Result<bool> {
    let Some(session) = session_for_pty(state, pty_id)? else {
        return Ok(false);
    };
    if session.runtime_state == RuntimeState::Stopping
        && session.stop_reason == Some(StopReason::IdleEvicted)
    {
        cancel_session(state, &session.pubkey, session.runtime_generation);
        let stopped = state.with_store(|store| {
            store.finalize_runtime_stopped_if_epoch(
                &session.pubkey,
                session.runtime_generation,
                session.lifecycle_epoch,
                StopReason::IdleEvicted,
                exited_at,
            )
        })?;
        state.with_store(|store| {
            store.clear_runtime_locator_if_generation(
                &session.pubkey,
                crate::state::LOCATOR_PTY,
                session.runtime_generation,
            )
        })?;
        if let Some(stopped) = stopped {
            super::presence::close_generation(
                state,
                &stopped.pubkey,
                stopped.runtime_generation,
                exited_at,
                "supervisor_stopped",
            )
            .await;
            emit_stopped(state, &stopped, exited_at);
            return Ok(true);
        }
        return Ok(false);
    }
    if session.runtime_state == RuntimeState::Stopped {
        state.with_store(|store| {
            store.clear_runtime_locator_if_generation(
                &session.pubkey,
                crate::state::LOCATOR_PTY,
                session.runtime_generation,
            )
        })?;
        return Ok(false);
    }
    let reason = match (child_success, presentation.attached_clients > 0) {
        (Some(true), true) => StopReason::AttachedCleanExit,
        (Some(false), _) | (None, _) => StopReason::Crash,
        (Some(true), false) => StopReason::HeadlessExit,
    };
    let stopped = stop_generation(state, &session, reason, exited_at).await?;
    state.with_store(|store| {
        store.clear_runtime_locator_if_generation(
            &session.pubkey,
            crate::state::LOCATOR_PTY,
            session.runtime_generation,
        )
    })?;
    if stopped && reason == StopReason::AttachedCleanExit {
        standing::reconcile_expired(state).await;
    }
    Ok(stopped)
}

pub(super) async fn stop_generation(
    state: &Arc<DaemonState>,
    session: &Session,
    reason: StopReason,
    stopped_at: u64,
) -> Result<bool> {
    stop_generation_locked(state, session, reason, stopped_at).await
}

async fn stop_generation_locked(
    state: &Arc<DaemonState>,
    session: &Session,
    reason: StopReason,
    stopped_at: u64,
) -> Result<bool> {
    cancel_session(state, &session.pubkey, session.runtime_generation);
    let changed = state.with_store(|store| {
        store.mark_runtime_stopped_if_generation(
            &session.pubkey,
            session.runtime_generation,
            reason,
            stopped_at,
        )
    })?;
    if !changed {
        return Ok(false);
    }
    let stopped = state
        .with_store(|store| store.get_session(&session.pubkey))?
        .context("stopped session disappeared")?;
    super::presence::close_generation(
        state,
        &stopped.pubkey,
        stopped.runtime_generation,
        stopped_at,
        "lifecycle_stopped",
    )
    .await;
    emit_stopped(state, &stopped, stopped_at);
    Ok(true)
}

fn session_for_pty(state: &Arc<DaemonState>, pty_id: &str) -> Result<Option<Session>> {
    state.with_store(|store| store.session_for_runtime_locator(crate::state::LOCATOR_PTY, pty_id))
}

fn emit_stopped(state: &Arc<DaemonState>, session: &Session, at: u64) {
    state.emit_tail(TailEvent::Sess {
        ts: at,
        channel: session.channel_h.clone(),
        agent: session.agent_slug.clone(),
        session: session.pubkey.clone(),
        state: "end".into(),
        rel_cwd: String::new(),
    });
}
