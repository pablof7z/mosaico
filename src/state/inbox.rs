//! `inbox` — durable inbound delivery state keyed by recipient pubkey.
//!
//! One row per (inbound event, target local agent). An event is "handled"
//! because a row exists. Direct-message rows start `pending` (parked for the next
//! hook), become `delivered` when surfaced by turn context, or `injected` when
//! submitted through a hosted PTY as a prompt awaiting echo suppression. Consumed echoes
//! become `echo_consumed`. Runtime ids are locators and never enter this ledger;
//! orchestration and management replay guards live in `event_claims`.
use super::*;

const COLS: &str = "event_id, target_pubkey, state, from_pubkey, channel_h, body, created_at, \
     delivered_at";
mod delivery;
mod prefix_lookup;

fn row_to_inbox(row: &rusqlite::Row) -> rusqlite::Result<InboxRow> {
    Ok(InboxRow {
        event_id: row.get(0)?,
        target_pubkey: row.get(1)?,
        state: row.get(2)?,
        from_pubkey: row.get(3)?,
        channel_h: row.get(4)?,
        body: row.get(5)?,
        created_at: row.get(6)?,
        delivered_at: row.get(7)?,
    })
}

impl Store {
    /// Record an inbound event addressed to a local agent pubkey. Idempotent: a
    /// duplicate `(event_id, target_pubkey)` is ignored. Returns `true` if newly
    /// enqueued.
    pub fn enqueue_inbox(
        &self,
        event_id: &str,
        target_pubkey: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        created_at: u64,
    ) -> Result<bool> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO inbox
                 (event_id, target_pubkey, state, from_pubkey, channel_h, body, created_at)
             VALUES (?1, ?2, 'pending', ?3, ?4, ?5, ?6)",
            params![
                event_id,
                target_pubkey,
                from_pubkey,
                channel_h,
                body,
                created_at
            ],
        )?;
        Ok(n > 0)
    }

    /// Mark a parked inbound event as delivered to its agent identity.
    pub fn mark_delivered(
        &self,
        event_id: &str,
        target_pubkey: &str,
        delivered_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE inbox SET state='delivered', delivered_at=?3
             WHERE event_id=?1 AND target_pubkey=?2",
            params![event_id, target_pubkey, delivered_at],
        )?;
        Ok(())
    }

    /// All pending inbound rows for an agent, oldest-first, WITHOUT consuming
    /// them — a read-only peek for callers that only display or warm caches
    /// (statusline, `who`, profile warm-up, the doorbell's "has pending?"
    /// filter). Delivery paths must use [`Store::claim_pending_for_pubkey`]
    /// instead, so the rows are claimed atomically.
    pub fn peek_pending_for_pubkey(&self, target_pubkey: &str) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             WHERE target_pubkey=?1 AND state='pending' ORDER BY created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target_pubkey], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// True if this (event, target) pair has already been recorded — the
    /// idempotency check.
    pub fn is_event_handled(&self, event_id: &str, target_pubkey: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM inbox WHERE event_id=?1 AND target_pubkey=?2",
                params![event_id, target_pubkey],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
#[path = "inbox/tests.rs"]
mod tests;
