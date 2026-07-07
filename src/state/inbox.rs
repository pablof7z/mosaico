//! `inbox` — the inbound routing ledger AND local inbound idempotency records.
//!
//! One row per (inbound event, target local session). An event is "handled"
//! because a row exists. Direct-message rows start `pending` (parked for the next
//! hook), become `delivered` when surfaced by turn context, or `injected` when
//! submitted through a hosted PTY as a prompt awaiting echo suppression. Consumed echoes
//! become `echo_consumed`. Orchestration target claims reuse the same ledger with
//! synthetic `target_session` keys: `processing` while a backend is mutating that
//! target, `pending` when it should be retried, and `delivered` once that exact
//! target is complete.
use super::*;

const COLS: &str = "event_id, target_session, state, from_pubkey, channel_h, body, created_at, \
     delivered_at";
const ORCHESTRATION_PROCESSING_LEASE_SECS: u64 = 10 * 60;
mod delivery;
mod prefix_lookup;

fn row_to_inbox(row: &rusqlite::Row) -> rusqlite::Result<InboxRow> {
    Ok(InboxRow {
        event_id: row.get(0)?,
        target_session: row.get(1)?,
        state: row.get(2)?,
        from_pubkey: row.get(3)?,
        channel_h: row.get(4)?,
        body: row.get(5)?,
        created_at: row.get(6)?,
        delivered_at: row.get(7)?,
    })
}

impl Store {
    /// Record an inbound event addressed to a local session. The target id is
    /// resolved to its canonical session first. Idempotent: a duplicate
    /// (event_id, target_session) is ignored. Returns `true` if newly enqueued.
    pub fn enqueue_inbox(
        &self,
        event_id: &str,
        target_session: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        created_at: u64,
    ) -> Result<bool> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO inbox
                 (event_id, target_session, state, from_pubkey, channel_h, body, created_at)
             VALUES (?1, ?2, 'pending', ?3, ?4, ?5, ?6)",
            params![event_id, target, from_pubkey, channel_h, body, created_at],
        )?;
        Ok(n > 0)
    }

    /// Mark a parked inbound event as delivered into a live session (resolves the
    /// target id first).
    pub fn mark_delivered(
        &self,
        event_id: &str,
        target_session: &str,
        delivered_at: u64,
    ) -> Result<()> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        self.conn.execute(
            "UPDATE inbox SET state='delivered', delivered_at=?3
             WHERE event_id=?1 AND target_session=?2",
            params![event_id, target, delivered_at],
        )?;
        Ok(())
    }

    /// All pending inbound rows for a session, oldest-first, WITHOUT consuming
    /// them — a read-only peek for callers that only display or warm caches
    /// (statusline, `who`, profile warm-up, the doorbell's "has pending?"
    /// filter). Delivery paths must use [`Store::claim_pending_for_session`]
    /// instead, so the rows are claimed atomically (resolves the id first).
    pub fn peek_pending_for_session(&self, target_session: &str) -> Result<Vec<InboxRow>> {
        // Fall back to the raw id when no canonical mapping exists — symmetric
        // with `enqueue_inbox`, which parks under the same raw id in that case.
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             WHERE target_session=?1 AND state='pending' ORDER BY created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// True if this (event, target) pair has already been recorded — the
    /// idempotency check (resolves the id first).
    pub fn is_event_handled(&self, event_id: &str, target_session: &str) -> Result<bool> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM inbox WHERE event_id=?1 AND target_session=?2",
                params![event_id, target],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Claim one backend orchestration target for processing. Returns `true`
    /// only when the caller should process it now. Completed targets and live
    /// in-flight claims return `false`; failed targets are returned to `pending`
    /// by [`Store::retry_orchestration_target`] and can be claimed again.
    pub fn claim_orchestration_target(
        &self,
        event_id: &str,
        target_key: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        now: u64,
    ) -> Result<bool> {
        let stale_before = now.saturating_sub(ORCHESTRATION_PROCESSING_LEASE_SECS);
        let n = self.conn.execute(
            "INSERT INTO inbox
                 (event_id, target_session, state, from_pubkey, channel_h, body, created_at, delivered_at)
             VALUES (?1, ?2, 'processing', ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(event_id, target_session) DO UPDATE SET
                 state='processing',
                 from_pubkey=excluded.from_pubkey,
                 channel_h=excluded.channel_h,
                 body=excluded.body,
                 delivered_at=excluded.delivered_at
             WHERE inbox.state='pending'
                OR (inbox.state='processing' AND inbox.delivered_at < ?7)",
            params![
                event_id,
                target_key,
                from_pubkey,
                channel_h,
                body,
                now,
                stale_before
            ],
        )?;
        Ok(n > 0)
    }

    pub fn complete_orchestration_target(
        &self,
        event_id: &str,
        target_key: &str,
        now: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE inbox SET state='delivered', delivered_at=?3
             WHERE event_id=?1 AND target_session=?2",
            params![event_id, target_key, now],
        )?;
        Ok(())
    }

    pub fn retry_orchestration_target(&self, event_id: &str, target_key: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE inbox SET state='pending', delivered_at=0
             WHERE event_id=?1 AND target_session=?2 AND state='processing'",
            params![event_id, target_key],
        )?;
        Ok(())
    }

    /// Claim one management command event for processing. Management commands
    /// are addressed to the backend key, not a local session, but they need the
    /// same durable replay guard as orchestration so a relay replay does not
    /// spawn or kill twice.
    pub fn claim_management_command(
        &self,
        event_id: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        now: u64,
    ) -> Result<bool> {
        self.claim_orchestration_target(event_id, "management", from_pubkey, channel_h, body, now)
    }

    pub fn complete_management_command(&self, event_id: &str, now: u64) -> Result<()> {
        self.complete_orchestration_target(event_id, "management", now)
    }
}

#[cfg(test)]
#[path = "inbox/tests.rs"]
mod tests;
