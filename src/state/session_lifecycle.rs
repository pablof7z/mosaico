//! Transactional reducer for persisted runtime lifecycle edges and deadlines.

use super::*;
use rusqlite::{Transaction, TransactionBehavior};

pub const HEADLESS_IDLE_TIMEOUT_SECS: u64 = 10 * 60;
pub const STOPPED_STANDING_RETENTION_SECS: u64 = 60 * 60;

impl Store {
    pub fn apply_session_presentation_edge(
        &self,
        pubkey: &str,
        generation: u64,
        attachment_epoch: u64,
        presentation: PresentationState,
        at: u64,
    ) -> Result<bool> {
        let idle_since = if presentation == PresentationState::Headless {
            at
        } else {
            0
        };
        let idle_deadline = idle_since.saturating_add(HEADLESS_IDLE_TIMEOUT_SECS);
        Ok(self.conn.execute(
            "UPDATE sessions
             SET presentation_state=?4, attachment_epoch=?3,
                 idle_since=CASE WHEN ?4='headless' AND work_state='idle' THEN ?5 ELSE 0 END,
                 idle_deadline=CASE WHEN ?4='headless' AND work_state='idle' THEN ?6 ELSE 0 END
             WHERE pubkey=?1 AND runtime_generation=?2 AND runtime_state='running'
               AND (attachment_epoch<?3 OR (
                   attachment_epoch=?3 AND presentation_state='unavailable'
               ))",
            params![
                pubkey,
                generation,
                attachment_epoch,
                presentation.as_str(),
                idle_since,
                idle_deadline
            ],
        )? == 1)
    }

    pub fn apply_session_turn_started(
        &self,
        pubkey: &str,
        generation: u64,
        at: u64,
        transcript_ref: Option<&str>,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE sessions
             SET turn_started_at=CASE WHEN work_state='idle' THEN ?3 ELSE turn_started_at END,
                 work_state='working',
                 turn_count=turn_count + CASE WHEN work_state='idle' THEN 1 ELSE 0 END,
                 state_changed_at=CASE WHEN work_state='idle' THEN ?3 ELSE state_changed_at END,
                 idle_since=0, idle_deadline=0,
                 transcript_path=COALESCE(?4, transcript_path)
             WHERE pubkey=?1 AND runtime_generation=?2 AND runtime_state='running'
            ",
            params![pubkey, generation, at, transcript_ref],
        )? == 1)
    }

    pub fn apply_session_turn_ended(&self, pubkey: &str, generation: u64, at: u64) -> Result<bool> {
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE sessions
             SET busy_seconds=busy_seconds + CASE
                     WHEN turn_started_at>0 THEN MAX(0, ?3-turn_started_at) ELSE 0 END,
                 work_state='idle', turn_started_at=0,
                 state_changed_at=?3,
                 idle_since=CASE WHEN presentation_state='headless' THEN ?3 ELSE 0 END,
                 idle_deadline=CASE WHEN presentation_state='headless' THEN ?4 ELSE 0 END
             WHERE pubkey=?1 AND runtime_generation=?2 AND runtime_state='running'
               AND work_state='working'",
            params![
                pubkey,
                generation,
                at,
                at.saturating_add(HEADLESS_IDLE_TIMEOUT_SECS)
            ],
        )?;
        if changed == 1 {
            transaction.execute(
                "UPDATE inbox SET state='echo_consumed'
                 WHERE target_pubkey=?1 AND state='injected'",
                [pubkey],
            )?;
        }
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn cancel_session_idle_deadline_for_delivery(
        &self,
        pubkey: &str,
        generation: u64,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE sessions SET idle_since=0, idle_deadline=0
             WHERE pubkey=?1 AND runtime_generation=?2 AND runtime_state='running'",
            params![pubkey, generation],
        )? == 1)
    }

    pub fn list_due_idle_evictions(&self, now: u64) -> Result<Vec<Session>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM sessions session
             WHERE runtime_state='running' AND presentation_state='headless'
               AND work_state='idle' AND idle_deadline>0 AND idle_deadline<=?1
               AND NOT EXISTS (
                   SELECT 1 FROM inbox item WHERE item.target_pubkey=session.pubkey
                     AND item.state IN ('pending', 'injected')
               )
             ORDER BY idle_deadline, pubkey",
            super::sessions::COLS
        ))?;
        let rows = statement.query_map([now], super::sessions::row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn reserve_due_idle_eviction(
        &self,
        pubkey: &str,
        generation: u64,
        lifecycle_epoch: u64,
        attachment_epoch: u64,
        now: u64,
    ) -> Result<Option<Session>> {
        let changed = self.conn.execute(
            "UPDATE sessions
             SET runtime_state='stopping', lifecycle_epoch=lifecycle_epoch+1,
                 idle_since=0, idle_deadline=0, stopped_at=?5,
                 stop_reason='idle_evicted', state_changed_at=?5
             WHERE pubkey=?1 AND runtime_generation=?2 AND lifecycle_epoch=?3
               AND attachment_epoch=?4 AND runtime_state='running'
               AND presentation_state='headless' AND work_state='idle'
               AND idle_deadline>0 AND idle_deadline<=?5
               AND NOT EXISTS (
                   SELECT 1 FROM inbox item WHERE item.target_pubkey=sessions.pubkey
                     AND item.state IN ('pending', 'injected')
               )",
            params![pubkey, generation, lifecycle_epoch, attachment_epoch, now],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.get_session(pubkey)
    }

    pub fn cancel_idle_eviction_on_presentation_change(
        &self,
        pubkey: &str,
        generation: u64,
        stopping_lifecycle_epoch: u64,
        attachment_epoch: u64,
        presentation: PresentationState,
        at: u64,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE sessions
             SET runtime_state='running', presentation_state=?5, stop_reason=NULL,
                 attachment_epoch=?4, lifecycle_epoch=lifecycle_epoch+1,
                 stopped_at=0, state_changed_at=?6,
                 idle_since=CASE WHEN ?5='headless' THEN ?6 ELSE 0 END,
                 idle_deadline=CASE WHEN ?5='headless' THEN ?7 ELSE 0 END
             WHERE pubkey=?1 AND runtime_generation=?2 AND runtime_state='stopping'
               AND lifecycle_epoch=?3 AND (
                   attachment_epoch<?4 OR (?5='unavailable' AND attachment_epoch=?4)
               )",
            params![
                pubkey,
                generation,
                stopping_lifecycle_epoch,
                attachment_epoch,
                presentation.as_str(),
                at,
                at.saturating_add(HEADLESS_IDLE_TIMEOUT_SECS)
            ],
        )? == 1)
    }

    pub fn finalize_runtime_stopped_if_epoch(
        &self,
        pubkey: &str,
        generation: u64,
        stopping_lifecycle_epoch: u64,
        reason: StopReason,
        stopped_at: u64,
    ) -> Result<Option<Session>> {
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE sessions
             SET runtime_state='stopped', presentation_state='unavailable', work_state='idle',
                 lifecycle_epoch=lifecycle_epoch+1, stopped_at=?5, stop_reason=?4,
                 busy_seconds=busy_seconds + CASE
                     WHEN work_state='working' AND turn_started_at>0
                     THEN MAX(0, ?5-turn_started_at) ELSE 0 END,
                 idle_since=0, idle_deadline=0, turn_started_at=0, state_changed_at=?5
             WHERE pubkey=?1 AND runtime_generation=?2 AND runtime_state='stopping'
               AND lifecycle_epoch=?3",
            params![
                pubkey,
                generation,
                stopping_lifecycle_epoch,
                reason.as_str(),
                stopped_at
            ],
        )?;
        if changed == 0 {
            transaction.rollback()?;
            return Ok(None);
        }
        transaction.execute(
            "UPDATE handle_leases SET live=0,
                 last_active_at=MAX(last_active_at, ?2) WHERE pubkey=?1",
            params![pubkey, stopped_at],
        )?;
        let lifecycle_epoch: u64 = transaction.query_row(
            "SELECT lifecycle_epoch FROM sessions WHERE pubkey=?1",
            [pubkey],
            |row| row.get(0),
        )?;
        super::session_standing::retain_in_transaction(
            &transaction,
            pubkey,
            lifecycle_epoch,
            stopped_at.saturating_add(STOPPED_STANDING_RETENTION_SECS),
            stopped_at,
        )?;
        transaction.commit()?;
        self.get_session(pubkey)
    }
}

#[cfg(test)]
#[path = "session_lifecycle/busy_tests.rs"]
mod busy_tests;
#[cfg(test)]
#[path = "session_lifecycle/tests.rs"]
mod tests;
