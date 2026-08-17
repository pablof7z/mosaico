use super::*;
use crate::fabric::provider::ConfirmedGroupScope;
use crate::state::ConfirmedAdmissionCommit;

mod repair;

pub(super) async fn reconcile_running(state: &Arc<DaemonState>) {
    repair::reconcile_running(state).await;
}

/// Revalidate a relay-admission task while `standing_sync` is held. Existing
/// durable routes are always authoritative. A fresh launch may establish a new
/// route only if this lifecycle has not already recorded an explicit absence.
pub(in crate::daemon::server) fn admission_is_current(
    state: &Arc<DaemonState>,
    pubkey: &str,
    channel: &str,
    runtime_generation: u64,
    lifecycle_epoch: u64,
    allow_new_route: bool,
) -> bool {
    state
        .with_store(|store| -> Result<bool> {
            let Some(session) = store.get_session(pubkey)? else {
                return Ok(false);
            };
            if !session.is_running()
                || session.runtime_generation != runtime_generation
                || session.lifecycle_epoch != lifecycle_epoch
            {
                return Ok(false);
            }
            if store.has_session_route(pubkey, channel)? {
                return Ok(true);
            }
            if !allow_new_route {
                return Ok(false);
            }
            Ok(store
                .get_session_standing(pubkey, channel)?
                .is_none_or(|standing| standing.session_lifecycle_epoch != lifecycle_epoch))
        })
        .unwrap_or_else(|error| {
            tracing::error!(
                pubkey = %pubkey_short(pubkey),
                %channel,
                %error,
                "admission authorization revalidation failed"
            );
            false
        })
}

/// Finalize relay-confirmed membership while the caller holds `standing_sync`.
/// The exact lifecycle may already have stopped; runtime stop is not leave.
/// A stale or failed primary commit first becomes durable cleanup work, so an
/// unconfirmed compensation is retried by the standing coordinator.
pub(in crate::daemon::server) async fn commit_confirmed_admission(
    state: &Arc<DaemonState>,
    pubkey: &str,
    channel: &str,
    runtime_generation: u64,
    lifecycle_epoch: u64,
) -> Result<bool> {
    let now = now_secs();
    let primary = state.with_store(|store| {
        store.commit_confirmed_session_admission(
            pubkey,
            channel,
            runtime_generation,
            lifecycle_epoch,
            now,
        )
    });
    match primary {
        Ok(ConfirmedAdmissionCommit::Committed) => {
            reconcile_admission(state, pubkey, channel, runtime_generation).await;
            Ok(true)
        }
        Ok(ConfirmedAdmissionCommit::Superseded) => {
            tracing::warn!(pubkey = %pubkey_short(pubkey), %channel, lifecycle_epoch, "stale admission was superseded by newer member standing");
            Ok(false)
        }
        Ok(ConfirmedAdmissionCommit::CleanupDue(due)) => {
            repair::compensate_due_admission(state, &due).await;
            Ok(false)
        }
        Err(primary_error) => {
            let fallback = state.with_store(|store| {
                store.schedule_confirmed_admission_cleanup(
                    pubkey,
                    channel,
                    runtime_generation,
                    lifecycle_epoch,
                    now_secs(),
                )
            });
            match fallback {
                Ok(ConfirmedAdmissionCommit::Committed) => {
                    tracing::warn!(pubkey = %pubkey_short(pubkey), %channel, %primary_error, "admission commit reported an error but its exact durable state is present");
                    reconcile_admission(state, pubkey, channel, runtime_generation).await;
                    Ok(true)
                }
                Ok(ConfirmedAdmissionCommit::Superseded) => {
                    tracing::warn!(pubkey = %pubkey_short(pubkey), %channel, %primary_error, "failed admission commit was superseded by newer member standing");
                    Ok(false)
                }
                Ok(ConfirmedAdmissionCommit::CleanupDue(due)) => {
                    repair::compensate_due_admission(state, &due).await;
                    Err(primary_error).context("confirmed admission could not be committed")
                }
                Err(cleanup_error) => Err(anyhow::anyhow!(
                    "confirmed admission commit failed ({primary_error:#}); durable cleanup persistence also failed ({cleanup_error:#})"
                )),
            }
        }
    }
}

async fn reconcile_admission(
    state: &Arc<DaemonState>,
    pubkey: &str,
    channel: &str,
    generation: u64,
) {
    super::super::presence::reassert_generation(
        state,
        pubkey,
        generation,
        "channel_admitted",
        Some(ConfirmedGroupScope::from_nmp_membership(channel, pubkey)),
    )
    .await;
}
