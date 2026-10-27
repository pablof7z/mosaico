//! Durable retry pacing for failed event claims.
//!
//! Every failure is one attempt. Attempt `n` waits `RETRY_BASE_DELAY_SECS *
//! 2^(n-1)`, capped at `RETRY_MAX_DELAY_SECS`; the `MAX_RETRY_ATTEMPTS`th
//! failure, or a failure the caller knows is permanent, is terminal `failed`.
//! `attempts` and `retry_at` are stored, so pacing survives a daemon restart.
//! Every transition to `failed` is logged here, whichever path caused it.

use super::*;
use std::collections::BTreeSet;

pub(crate) const RETRY_BASE_DELAY_SECS: u64 = 30;
pub(crate) const RETRY_MAX_DELAY_SECS: u64 = 60 * 60;
pub(crate) const MAX_RETRY_ATTEMPTS: u32 = 10;
/// Claim-key prefixes whose events the demux retry driver re-dispatches.
pub(crate) const ORCHESTRATION_CLAIM_PREFIX: &str = "orchestration:";
pub(crate) const SESSION_DISPATCH_CLAIM_PREFIX: &str = "session-dispatch:";
/// Orchestration and session-dispatch claims, re-dispatched from the view.
const EVENT_DRIVEN: &str =
    "(claim_key LIKE 'orchestration:%' OR claim_key LIKE 'session-dispatch:%')";
/// Every claim the demux retry timer wakes for, including offline mentions,
/// which recover from their stored claim payload.
const TIMER_DRIVEN: &str = "(claim_key LIKE 'orchestration:%' \
     OR claim_key LIKE 'session-dispatch:%' OR claim_key LIKE 'offline-mention:%')";
/// When a reclaimable row becomes due: a `pending` retry at `retry_at`, an
/// abandoned `processing` lease once it expires.
const DUE_AT: &str = "CASE state WHEN 'pending' THEN retry_at ELSE updated_at + ?1 + 1 END";

/// One reclaimable claim and when it is due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DueClaim {
    pub(crate) event_id: String,
    pub(crate) claim_key: String,
    pub(crate) due: u64,
}

/// What a recorded failure did to its claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClaimRetry {
    /// `pending` again, claimable at `retry_at`.
    Scheduled { attempts: u32, retry_at: u64 },
    /// Out of attempts: terminal `failed`.
    Exhausted { attempts: u32 },
}

/// Delay before retry after the `attempts`th failure.
pub(crate) fn retry_delay_secs(attempts: u32) -> u64 {
    let doublings = attempts.saturating_sub(1).min(20);
    (RETRY_BASE_DELAY_SECS << doublings).min(RETRY_MAX_DELAY_SECS)
}

impl Store {
    /// Record a failed attempt for an in-flight claim. `None` means the claim
    /// was not `processing` (already finished or re-claimed elsewhere).
    pub(crate) fn retry_orchestration_target(
        &self,
        event_id: &str,
        claim_key: &str,
        now: u64,
    ) -> Result<Option<ClaimRetry>> {
        let attempts = self
            .conn
            .query_row(
                "SELECT attempts FROM event_claims
                 WHERE event_id=?1 AND claim_key=?2 AND state='processing'",
                params![event_id, claim_key],
                |row| row.get::<_, u32>(0),
            )
            .optional()?;
        let Some(attempts) = attempts else {
            return Ok(None);
        };
        self.charge_attempt(event_id, claim_key, attempts, now, "out of retry attempts")
            .map(Some)
    }

    /// Terminally fail a claim whose error cannot succeed on retry.
    pub(crate) fn fail_orchestration_target(
        &self,
        event_id: &str,
        claim_key: &str,
        now: u64,
        reason: &str,
    ) -> Result<()> {
        let failed = self.conn.execute(
            "UPDATE event_claims SET state='failed', updated_at=?3
             WHERE event_id=?1 AND claim_key=?2 AND state IN ('pending', 'processing')",
            params![event_id, claim_key, now],
        )?;
        if failed > 0 {
            log_terminal(event_id, claim_key, None, reason);
        }
        Ok(())
    }

    /// Earliest due time among timer-driven claims. Orchestration and
    /// session-dispatch claims of a `parked` event (absent from the current
    /// view) are skipped; offline mentions never depend on the view.
    pub(crate) fn next_claim_due_excluding(
        &self,
        parked: &BTreeSet<String>,
    ) -> Result<Option<u64>> {
        Ok(self.earliest_due_claim(parked)?.map(|claim| claim.due))
    }

    /// The earliest-due timer-driven claim, skipping parked events' view-driven
    /// claims. The retry driver compares it across a pass to detect progress.
    pub(crate) fn earliest_due_claim(&self, parked: &BTreeSet<String>) -> Result<Option<DueClaim>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT event_id, claim_key, {EVENT_DRIVEN} AS from_view, {DUE_AT} AS due
             FROM event_claims
             WHERE {TIMER_DRIVEN} AND state IN ('pending', 'processing')
             ORDER BY due, event_id, claim_key"
        ))?;
        let mut rows = statement.query(params![PROCESSING_LEASE_SECS])?;
        while let Some(row) = rows.next()? {
            let event_id: String = row.get(0)?;
            let from_view: bool = row.get(2)?;
            if !(from_view && parked.contains(&event_id)) {
                return Ok(Some(DueClaim {
                    event_id,
                    claim_key: row.get(1)?,
                    due: row.get(3)?,
                }));
            }
        }
        Ok(None)
    }

    /// Orchestration and session-dispatch events with a claim reclaimable at
    /// `now`, earliest first, skipping `parked` events.
    pub(crate) fn due_driven_claim_events(
        &self,
        now: u64,
        parked: &BTreeSet<String>,
    ) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT event_id FROM event_claims
             WHERE {EVENT_DRIVEN} AND state IN ('pending', 'processing')
               AND {DUE_AT} <= ?2
             GROUP BY event_id
             ORDER BY MIN({DUE_AT}), event_id"
        ))?;
        let rows = statement.query_map(params![PROCESSING_LEASE_SECS, now], |row| {
            row.get::<_, String>(0)
        })?;
        let mut due = Vec::new();
        for event_id in rows {
            let event_id = event_id?;
            if !parked.contains(&event_id) {
                due.push(event_id);
            }
        }
        Ok(due)
    }

    /// The event's orchestration and session-dispatch claim keys, any state
    /// but terminal or completed.
    pub(crate) fn open_driven_claim_keys(&self, event_id: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT claim_key FROM event_claims
             WHERE event_id=?1 AND {EVENT_DRIVEN} AND state IN ('pending', 'processing')"
        ))?;
        let rows = statement.query_map([event_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Charge one attempt to each due claim (a `pending` retry or an expired
    /// `processing` lease) of an event the converged view does not hold, so a
    /// vanished event backs off toward `failed` instead of staying due forever.
    pub(crate) fn charge_absent_driven_claims(&self, event_id: &str, now: u64) -> Result<()> {
        let due = {
            let mut statement = self.conn.prepare(&format!(
                "SELECT claim_key, attempts FROM event_claims
                 WHERE event_id=?2 AND {EVENT_DRIVEN}
                   AND state IN ('pending', 'processing') AND {DUE_AT} <= ?3"
            ))?;
            let rows = statement
                .query_map(params![PROCESSING_LEASE_SECS, event_id, now], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (key, attempts) in due {
            self.charge_attempt(
                event_id,
                &key,
                attempts,
                now,
                "event stayed absent from the view through every retry",
            )?;
        }
        Ok(())
    }

    /// Count one more failed attempt: `pending` at the next backoff, or
    /// terminal `failed` once attempts run out.
    fn charge_attempt(
        &self,
        event_id: &str,
        claim_key: &str,
        attempts: u32,
        now: u64,
        exhausted_reason: &str,
    ) -> Result<ClaimRetry> {
        let attempts = attempts.saturating_add(1);
        let retry = if attempts >= MAX_RETRY_ATTEMPTS {
            log_terminal(event_id, claim_key, Some(attempts), exhausted_reason);
            ClaimRetry::Exhausted { attempts }
        } else {
            ClaimRetry::Scheduled {
                attempts,
                retry_at: now + retry_delay_secs(attempts),
            }
        };
        let (state, retry_at) = match retry {
            ClaimRetry::Scheduled { retry_at, .. } => ("pending", retry_at),
            ClaimRetry::Exhausted { .. } => ("failed", 0),
        };
        self.conn.execute(
            "UPDATE event_claims SET state=?3, attempts=?4, retry_at=?5, updated_at=?6
             WHERE event_id=?1 AND claim_key=?2",
            params![event_id, claim_key, state, attempts, retry_at, now],
        )?;
        Ok(retry)
    }
}

pub(super) fn log_terminal(event_id: &str, claim_key: &str, attempts: Option<u32>, reason: &str) {
    tracing::warn!(
        event_id = %&event_id[..event_id.len().min(8)],
        claim = %claim_key,
        attempts,
        reason,
        "event claim failed permanently; it will not be retried"
    );
}

#[cfg(test)]
#[path = "retry/tests.rs"]
mod tests;
