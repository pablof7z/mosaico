//! Run ownership of event claims: the generation a claim records, the lease
//! renewal a run makes before each side effect, and the finish fence that
//! keeps a superseded run from overwriting the run that took over.

use super::*;

impl Store {
    /// Whether the run that claimed at `generation` (the `attempts` value its
    /// claim left) still owns the claim. A lease takeover or a later retry
    /// moves `attempts` on, so a superseded run cannot finish the claim.
    pub(crate) fn owns_claim(
        &self,
        event_id: &str,
        claim_key: &str,
        generation: u32,
    ) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM event_claims
                 WHERE event_id=?1 AND claim_key=?2 AND state='processing' AND attempts=?3)",
            params![event_id, claim_key, generation],
            |row| row.get(0),
        )?)
    }

    /// Extend a run's lease right before its side effect. `false` means the
    /// run no longer owns the claim (its lease was taken over, or the claim was
    /// charged or finished elsewhere) and must skip the side effect.
    pub(crate) fn renew_claim(
        &self,
        event_id: &str,
        claim_key: &str,
        generation: u32,
        now: u64,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE event_claims SET updated_at=?4
             WHERE event_id=?1 AND claim_key=?2 AND state='processing' AND attempts=?3",
            params![event_id, claim_key, generation, now],
        )? > 0)
    }

    /// Renew a run's lease on its current target and on every target it still
    /// holds queued behind it, in one store call. Returns whether the run
    /// still owns the current target (first in `held`); a queued target it no
    /// longer owns is skipped when its turn comes.
    pub(crate) fn renew_held_claims(
        &self,
        event_id: &str,
        held: &[(&str, u32)],
        now: u64,
    ) -> Result<bool> {
        let mut owns_current = false;
        for (position, (claim_key, generation)) in held.iter().enumerate() {
            let renewed = self.renew_claim(event_id, claim_key, *generation, now)?;
            if position == 0 {
                owns_current = renewed;
            }
        }
        Ok(owns_current)
    }

    /// Record a run's success unless a newer run now holds the claim. A run
    /// whose lease was charged while it was still working (now pending or
    /// failed) did the work, so its `Completed` wins over the charge.
    pub(crate) fn complete_unless_reclaimed(
        &self,
        event_id: &str,
        claim_key: &str,
        generation: u32,
        now: u64,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE event_claims SET state='completed', updated_at=?4
             WHERE event_id=?1 AND claim_key=?2
               AND ((state='processing' AND attempts=?3) OR state IN ('pending', 'failed'))",
            params![event_id, claim_key, generation, now],
        )? > 0)
    }

    /// Generation (`attempts`) of a claim this connection just took.
    pub(crate) fn claim_generation(&self, event_id: &str, claim_key: &str) -> Result<u32> {
        Ok(self.conn.query_row(
            "SELECT attempts FROM event_claims WHERE event_id=?1 AND claim_key=?2",
            params![event_id, claim_key],
            |row| row.get(0),
        )?)
    }

    /// Claim a driven target; `Some(generation)` when this call now owns it.
    pub(crate) fn claim_driven_target(
        &self,
        event_id: &str,
        claim_key: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        now: u64,
    ) -> Result<Option<u32>> {
        if !self.claim_orchestration_target(
            event_id,
            claim_key,
            from_pubkey,
            channel_h,
            body,
            now,
        )? {
            return Ok(None);
        }
        self.claim_generation(event_id, claim_key).map(Some)
    }
}

#[cfg(test)]
#[path = "ownership/tests.rs"]
mod tests;
