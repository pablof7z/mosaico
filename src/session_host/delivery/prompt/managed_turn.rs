//! Durable lifecycle and native-outcome ownership for RPC-delivered turns.

use crate::daemon::server::DaemonState;
use crate::session_host::transport::DeliveryCompletion;
use crate::util::now_secs;
use anyhow::Result;
use std::sync::Arc;

/// RPC transports own their turn boundary, unlike PTY transports whose native
/// hooks project it. Start the lifecycle and attempt ledger after inbox commit,
/// then close both from exact RPC completion evidence.
pub(super) async fn track(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    event_ids: &[String],
    delivery_kind: crate::state::NativeTurnDeliveryKind,
    trigger_event_id: &str,
    completion: DeliveryCompletion,
) -> Result<()> {
    let (native_thread_id, completion) = match completion {
        DeliveryCompletion::ExternallyObserved => return Ok(()),
        DeliveryCompletion::Managed {
            native_thread_id,
            completion,
        } => (native_thread_id, completion),
        DeliveryCompletion::ManagedSteer(accepted) => {
            track_steer(state, rec, event_ids, accepted);
            return Ok(());
        }
    };
    let started_at = now_secs();
    let attempt_id = state.with_store(|store| {
        store.start_native_turn_attempt(&crate::state::NewNativeTurnAttempt {
            pubkey: &rec.pubkey,
            runtime_generation: rec.runtime_generation,
            delivery_kind,
            delivery_event_id: trigger_event_id,
            native_thread_id: &native_thread_id,
            started_at,
        })
    })?;
    let started = state.with_store(|store| {
        store.apply_session_turn_started(&rec.pubkey, rec.runtime_generation, started_at)
    })?;
    if !started {
        finalize_stopped_generation(state, rec, attempt_id);
        anyhow::bail!(
            "RPC turn started after session {} generation {} stopped",
            rec.pubkey,
            rec.runtime_generation
        );
    }
    crate::daemon::server::presence::reconcile_generation(
        state,
        &rec.pubkey,
        rec.runtime_generation,
        "managed_turn_started",
        None,
    )
    .await;
    crate::daemon::server::turns::work_start_reaction::publish_for_started_events(
        state, rec, event_ids,
    );

    let state = state.clone();
    let pubkey = rec.pubkey.clone();
    let generation = rec.runtime_generation;
    tokio::spawn(async move {
        let result = completion.await.unwrap_or_else(|_| {
            crate::session_host::transport::ManagedTurnResult {
                native_turn_id: String::new(),
                outcome: crate::state::NativeTurnOutcome::UnknownReconciled,
                error_message: "managed RPC completion sender was dropped".into(),
                error_details: String::new(),
            }
        });
        persist_outcome(&state, &pubkey, generation, attempt_id, &result);
        finish_lifecycle(state, pubkey, generation).await;
    });
    Ok(())
}

fn track_steer(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    event_ids: &[String],
    accepted: tokio::sync::oneshot::Receiver<anyhow::Result<()>>,
) {
    let state = state.clone();
    let rec = rec.clone();
    let event_ids = event_ids.to_vec();
    tokio::spawn(async move {
        match accepted.await {
            Ok(Ok(())) => {
                crate::daemon::server::turns::work_start_reaction::publish_for_started_events(
                    &state, &rec, &event_ids,
                );
            }
            Ok(Err(error)) => tracing::warn!(
                session = %rec.pubkey, %error,
                "app-server steer was not accepted; work-start reaction skipped"
            ),
            Err(_) => tracing::warn!(
                session = %rec.pubkey,
                "app-server steer confirmation was dropped; work-start reaction skipped"
            ),
        }
    });
}

fn finalize_stopped_generation(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    attempt_id: i64,
) {
    let _ = state.with_store(|store| {
        store.finish_native_turn_attempt(&crate::state::FinishNativeTurnAttempt {
            id: attempt_id,
            pubkey: &rec.pubkey,
            runtime_generation: rec.runtime_generation,
            native_turn_id: "",
            outcome: crate::state::NativeTurnOutcome::UnknownReconciled,
            error_message: "session generation stopped after native delivery",
            error_details: "",
            finished_at: now_secs(),
        })
    });
}

fn persist_outcome(
    state: &Arc<DaemonState>,
    pubkey: &str,
    generation: u64,
    attempt_id: i64,
    result: &crate::session_host::transport::ManagedTurnResult,
) {
    if result.outcome.is_failure() {
        tracing::warn!(
            session = pubkey,
            outcome = result.outcome.as_str(),
            error = %result.error_message,
            "managed RPC turn ended with an error"
        );
    }
    match state.with_store(|store| {
        store.finish_native_turn_attempt(&crate::state::FinishNativeTurnAttempt {
            id: attempt_id,
            pubkey,
            runtime_generation: generation,
            native_turn_id: &result.native_turn_id,
            outcome: result.outcome,
            error_message: &result.error_message,
            error_details: &result.error_details,
            finished_at: now_secs(),
        })
    }) {
        Ok(true) => {}
        Ok(false) => tracing::warn!(
            session = pubkey,
            generation,
            attempt_id,
            "native turn outcome was already finalized"
        ),
        Err(error) => tracing::error!(
            session = pubkey, generation, attempt_id, %error,
            "failed to persist native turn outcome"
        ),
    }
}

async fn finish_lifecycle(state: Arc<DaemonState>, pubkey: String, generation: u64) {
    match state.with_store(|store| store.apply_session_turn_ended(&pubkey, generation, now_secs()))
    {
        Ok(true) => {
            crate::daemon::server::presence::reconcile_generation(
                &state,
                &pubkey,
                generation,
                "managed_turn_ended",
                None,
            )
            .await;
            crate::session_host::ring_doorbells(state)
        }
        Ok(false) => tracing::debug!(
            session = %pubkey, generation,
            "managed RPC completion was superseded by a lifecycle edge"
        ),
        Err(error) => tracing::error!(
            session = %pubkey, generation, %error,
            "failed to project managed RPC turn completion"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AdmittedRuntimeFacts, RegisterSession, WorkState};

    #[tokio::test]
    async fn terminal_failure_closes_lifecycle_and_persists_exact_correlation() {
        let state = DaemonState::new_for_test().await;
        let generation = state
            .with_store(|store| {
                store.reserve_session_with_facts(
                    &RegisterSession {
                        pubkey: "pk".into(),
                        observed_harness: "codex".into(),
                        agent_slug: "agent".into(),
                        launch_channel_h: "root".into(),
                        work_root: "root".into(),
                        child_pid: None,
                        now: 1,
                    },
                    &AdmittedRuntimeFacts {
                        observed_harness: "codex".into(),
                        claimed_harness: String::new(),
                        preset: String::new(),
                        transport: "app-server".into(),
                        endpoint_provenance: "launch".into(),
                    },
                )
            })
            .unwrap();
        let rec = state
            .with_store(|store| store.get_session("pk"))
            .unwrap()
            .unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();

        track(
            &state,
            &rec,
            &[],
            crate::state::NativeTurnDeliveryKind::InboxEvent,
            "event",
            DeliveryCompletion::Managed {
                native_thread_id: "thread".into(),
                completion: receiver,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            state
                .with_store(|store| store.get_session("pk"))
                .unwrap()
                .unwrap()
                .work_state,
            WorkState::Working
        );

        sender
            .send(crate::session_host::transport::ManagedTurnResult {
                native_turn_id: "turn".into(),
                outcome: crate::state::NativeTurnOutcome::Failed,
                error_message: "model rejected".into(),
                error_details: String::new(),
            })
            .unwrap();
        for _ in 0..50 {
            if state
                .with_store(|store| store.get_session("pk"))
                .unwrap()
                .is_some_and(|session| session.work_state == WorkState::Idle)
            {
                break;
            }
            tokio::task::yield_now().await;
        }

        let session = state
            .with_store(|store| store.get_session("pk"))
            .unwrap()
            .unwrap();
        assert_eq!(session.work_state, WorkState::Idle);
        let outcome = state
            .with_store(|store| store.latest_native_turn_attempt("pk", generation))
            .unwrap()
            .unwrap();
        assert_eq!(outcome.delivery_event_id, "event");
        assert_eq!(outcome.native_thread_id, "thread");
        assert_eq!(outcome.native_turn_id, "turn");
        assert_eq!(outcome.outcome, crate::state::NativeTurnOutcome::Failed);
    }
}
