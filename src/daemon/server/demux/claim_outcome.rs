//! How a claimed orchestration or session-dispatch run is executed and
//! recorded: the lease renewal before its side effect, its outcome, and the
//! retry-timer wake after a retryable failure.

use super::*;
use crate::state::ClaimRetry;

/// How a handler's work on one claimed target ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::daemon::server) enum ClaimOutcome {
    Completed,
    /// May succeed later; retried with backoff until attempts run out.
    Retry(&'static str),
    /// Cannot succeed on retry (for example an unknown agent slug).
    Failed(&'static str),
}

impl From<bool> for ClaimOutcome {
    fn from(completed: bool) -> Self {
        if completed {
            Self::Completed
        } else {
            Self::Retry("handler did not complete")
        }
    }
}

/// Run one claimed target's side effect only while this run still owns the
/// claim, then finish it. Immediately before `work`, the lease is renewed for
/// this target and for every target the run still holds `queued` behind it
/// (an orchestration claims all targets up front and runs them in turn), so a
/// long serial run neither loses its later targets to a takeover nor runs a
/// target twice. Returns whether `work` ran.
pub(in crate::daemon::server) async fn run_owned(
    state: &Arc<DaemonState>,
    event_id: &str,
    claim_key: &str,
    generation: u32,
    queued: &[(&str, u32)],
    work: impl std::future::Future<Output = ClaimOutcome>,
) -> bool {
    let held: Vec<(&str, u32)> = std::iter::once((claim_key, generation))
        .chain(queued.iter().copied())
        .collect();
    match state.with_store(|s| s.renew_held_claims(event_id, &held, now_secs())) {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(event_id = %&event_id[..event_id.len().min(8)], claim = %claim_key, generation, "claim was taken over before its side effect; skipping");
            return false;
        }
        Err(error) => {
            // Without a renewed lease the side effect could run twice; let the
            // claim's lease expire and be retried instead.
            tracing::error!(event_id = %&event_id[..event_id.len().min(8)], claim = %claim_key, %error, "claim lease renewal failed; skipping");
            return false;
        }
    }
    let outcome = work.await;
    finish_claim(state, event_id, claim_key, generation, outcome);
    true
}

/// Record a claimed target's outcome and, after a retryable failure, wake the
/// demux loop so it re-arms its timer for the new due time. `generation` is
/// what the run's claim returned: a run whose lease was taken over (or that
/// was otherwise superseded) no longer owns the claim and changes nothing,
/// except that its `Completed` still wins over a charge (no newer run holds
/// the claim), because the work was done.
pub(in crate::daemon::server) fn finish_claim(
    state: &Arc<DaemonState>,
    event_id: &str,
    claim_key: &str,
    generation: u32,
    outcome: ClaimOutcome,
) {
    let short = &event_id[..event_id.len().min(8)];
    let now = now_secs();
    let result = state.with_store(|s| {
        let superseded = |s: &crate::state::Store| -> Result<bool> {
            let owns = s.owns_claim(event_id, claim_key, generation)?;
            if !owns {
                tracing::warn!(event_id = %short, claim = %claim_key, generation, ?outcome, "superseded run finished after its lease was taken over; outcome ignored");
            }
            Ok(!owns)
        };
        match outcome {
            ClaimOutcome::Completed => {
                if !s.complete_unless_reclaimed(event_id, claim_key, generation, now)? {
                    tracing::warn!(event_id = %short, claim = %claim_key, generation, "superseded run completed while a newer run holds the claim; outcome ignored");
                }
                Ok(None)
            }
            ClaimOutcome::Failed(reason) => {
                if !superseded(s)? {
                    s.fail_orchestration_target(event_id, claim_key, now, reason)?;
                }
                Ok(None)
            }
            ClaimOutcome::Retry(_) => {
                if superseded(s)? {
                    return Ok(None);
                }
                s.retry_orchestration_target(event_id, claim_key, now)
            }
        }
    });
    match result {
        Ok(Some(ClaimRetry::Scheduled { attempts, retry_at })) => {
            let ClaimOutcome::Retry(reason) = outcome else {
                unreachable!("only a retry schedules one")
            };
            tracing::info!(event_id = %short, claim = %claim_key, attempts, retry_at, reason, "claimed target failed; retry scheduled");
            retry_scheduled(state);
        }
        // Terminal transitions are logged by the store.
        Ok(Some(ClaimRetry::Exhausted { .. }) | None) => {}
        Err(error) => {
            tracing::error!(event_id = %short, claim = %claim_key, %error, "claim state update failed");
        }
    }
}

/// Wake the demux loop to re-arm its retry timer for a newly stored due time.
pub(in crate::daemon::server) fn retry_scheduled(state: &Arc<DaemonState>) {
    state.dedup.claim_retry_scheduled.notify_one();
}
