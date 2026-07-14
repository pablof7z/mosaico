//! Session distill-draft state: the locally-distilled title/activity draft plus
//! the failure-streak / heads-up throttle bookkeeping that drives the
//! agent-facing "status generation is failing" notice in `turn_context::start`.

use super::*;

impl Store {
    /// Update the locally-distilled pre-publish draft (title/activity) and stamp
    /// the distill time (resolves first). A success clears any in-progress
    /// failure streak, so a transient blip doesn't linger toward the
    /// agent-facing heads-up threshold.
    pub fn set_session_distill(
        &self,
        pubkey: &str,
        title: &str,
        activity: &str,
        last_distill_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET title=?2, activity=?3, last_distill_at=?4, distill_fail_streak=0 \
             WHERE pubkey=?1",
            params![pubkey, title, activity, last_distill_at],
        )?;
        Ok(())
    }

    /// Record one failed status-title generation attempt (resolves first).
    /// Consecutive failures accumulate; a success ([`Self::set_session_distill`])
    /// resets the streak to 0.
    pub fn record_distill_failure(&self, pubkey: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET distill_fail_streak = distill_fail_streak + 1 WHERE pubkey=?1",
            [pubkey],
        )?;
        Ok(())
    }

    /// Stamp the moment the agent-facing "status generation is failing" heads-up
    /// was last injected, throttling how often `turn_context::start` re-emits it
    /// while the failure streak persists (resolves first).
    pub fn mark_distill_notice(&self, pubkey: &str, at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET distill_notice_at=?2 WHERE pubkey=?1",
            params![pubkey, at],
        )?;
        Ok(())
    }
}
