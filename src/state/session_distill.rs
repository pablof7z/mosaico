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
        id: &str,
        title: &str,
        activity: &str,
        last_distill_at: u64,
    ) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET title=?2, activity=?3, last_distill_at=?4, distill_fail_streak=0 \
             WHERE session_id=?1",
            params![canonical, title, activity, last_distill_at],
        )?;
        Ok(())
    }

    /// Record one failed status-title generation attempt (resolves first).
    /// Consecutive failures accumulate; a success ([`Self::set_session_distill`])
    /// resets the streak to 0.
    pub fn record_distill_failure(&self, id: &str) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET distill_fail_streak = distill_fail_streak + 1 WHERE session_id=?1",
            params![canonical],
        )?;
        Ok(())
    }

    /// Stamp the moment the agent-facing "status generation is failing" heads-up
    /// was last injected, throttling how often `turn_context::start` re-emits it
    /// while the failure streak persists (resolves first).
    pub fn mark_distill_notice(&self, id: &str, at: u64) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET distill_notice_at=?2 WHERE session_id=?1",
            params![canonical, at],
        )?;
        Ok(())
    }
}
