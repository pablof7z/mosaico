use super::{row_to_inbox, COLS};
use crate::state::*;

impl Store {
    /// Atomically claim every pending row for an agent: flip each to
    /// `delivered` AND return it in a single statement. The FIRST caller - the
    /// direct injection path or a hook - wins; any concurrent caller gets an
    /// empty vec. This atomicity IS the dedup: a message can only be injected
    /// once, with no separate "notified" flag or external gate. Rows come back
    /// oldest-first (RETURNING order is unspecified, so we sort).
    pub fn claim_pending_for_pubkey(&self, target_pubkey: &str, now: u64) -> Result<Vec<InboxRow>> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut stmt = transaction.prepare(&format!(
            "UPDATE inbox SET state='delivered', delivered_at=?2
             WHERE target_pubkey=?1 AND state='pending'
             RETURNING {COLS}"
        ))?;
        let rows = stmt.query_map(params![target_pubkey, now], row_to_inbox)?;
        let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        out.sort_by_key(|r| r.created_at);
        drop(stmt);
        crate::state::work_start::stage_from_inbox_tx(&transaction, &out, now)?;
        transaction.commit()?;
        Ok(out)
    }

    /// Atomically claim only the specified pending event ids for an agent.
    /// The delivery reconciler plans against exact inbox ids; this applies that
    /// plan without consuming rows that arrived after the scan.
    pub fn claim_pending_event_ids_for_pubkey(
        &self,
        event_ids: &[String],
        target_pubkey: &str,
        now: u64,
    ) -> Result<Vec<InboxRow>> {
        let mut out = Vec::new();
        for id in event_ids {
            let mut stmt = self.conn.prepare(&format!(
                "UPDATE inbox SET state='delivered', delivered_at=?3
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='pending'
                 RETURNING {COLS}"
            ))?;
            let rows = stmt.query_map(params![id, target_pubkey, now], row_to_inbox)?;
            out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        out.sort_by_key(|r| r.created_at);
        Ok(out)
    }

    /// Roll claimed rows back to `pending` so they are retried rather than lost.
    /// Used only when direct injection fails AFTER the atomic claim.
    pub fn reenqueue_pending(&self, event_ids: &[String], target_pubkey: &str) -> Result<()> {
        for id in event_ids {
            self.conn.execute(
                "UPDATE inbox SET state='pending', delivered_at=0
                 WHERE event_id=?1 AND target_pubkey=?2",
                params![id, target_pubkey],
            )?;
        }
        Ok(())
    }

    /// Completed inbound rows for an agent whose delivery is newer than
    /// `since`, oldest-first. Powers statusline/integration peeks.
    pub fn recently_delivered_for_pubkey(
        &self,
        target_pubkey: &str,
        since: u64,
    ) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             WHERE target_pubkey=?1
               AND state IN ('delivered', 'injected', 'echo_consumed')
               AND delivered_at>=?2
             ORDER BY created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target_pubkey, since], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Mark successfully injected rows as awaiting user-prompt echo
    /// suppression. These rows are no longer pending for turn context delivery.
    pub fn mark_injected_for_echo(
        &self,
        event_ids: &[String],
        target_pubkey: &str,
        now: u64,
    ) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        for id in event_ids {
            transaction.execute(
                "UPDATE inbox SET state='injected'
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='delivered'",
                params![id, target_pubkey],
            )?;
        }
        let mut rows = Vec::new();
        for id in event_ids {
            let mut stmt = transaction.prepare(&format!(
                "SELECT {COLS} FROM inbox
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='injected'"
            ))?;
            let claimed = stmt.query_map(params![id, target_pubkey], row_to_inbox)?;
            rows.extend(claimed.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        crate::state::work_start::stage_from_inbox_tx(&transaction, &rows, now)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn injected_for_pubkey(&self, target_pubkey: &str) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             WHERE target_pubkey=?1 AND state='injected'
             ORDER BY delivered_at ASC, created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target_pubkey], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn consume_injected_echo(&self, event_ids: &[String], target_pubkey: &str) -> Result<()> {
        for id in event_ids {
            self.conn.execute(
                "UPDATE inbox SET state='echo_consumed'
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='injected'",
                params![id, target_pubkey],
            )?;
        }
        Ok(())
    }
}
