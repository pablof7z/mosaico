use super::{row_to_inbox, COLS};
use crate::state::*;

impl Store {
    /// Atomically claim every pending row for a session: flip each to
    /// `delivered` AND return it in a single statement. The FIRST caller - the
    /// direct injection path or a hook - wins; any concurrent caller gets an
    /// empty vec. This atomicity IS the dedup: a message can only be injected
    /// once, with no separate "notified" flag or external gate. Rows come back
    /// oldest-first (RETURNING order is unspecified, so we sort). Resolves the
    /// id first.
    pub fn claim_pending_for_session(
        &self,
        target_session: &str,
        now: u64,
    ) -> Result<Vec<InboxRow>> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        let mut stmt = self.conn.prepare(&format!(
            "UPDATE inbox SET state='delivered', delivered_at=?2
             WHERE target_session=?1 AND state='pending'
             RETURNING {COLS}"
        ))?;
        let rows = stmt.query_map(params![target, now], row_to_inbox)?;
        let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        out.sort_by_key(|r| r.created_at);
        Ok(out)
    }

    /// Atomically claim only the specified pending event ids for a session.
    /// The delivery reconciler plans against exact inbox ids; this applies that
    /// plan without consuming rows that arrived after the scan.
    pub fn claim_pending_event_ids_for_session(
        &self,
        event_ids: &[String],
        target_session: &str,
        now: u64,
    ) -> Result<Vec<InboxRow>> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        let mut out = Vec::new();
        for id in event_ids {
            let mut stmt = self.conn.prepare(&format!(
                "UPDATE inbox SET state='delivered', delivered_at=?3
                 WHERE event_id=?1 AND target_session=?2 AND state='pending'
                 RETURNING {COLS}"
            ))?;
            let rows = stmt.query_map(params![id, &target, now], row_to_inbox)?;
            out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        out.sort_by_key(|r| r.created_at);
        Ok(out)
    }

    /// Roll claimed rows back to `pending` so they are retried rather than lost.
    /// Used only when direct injection fails AFTER the atomic claim.
    pub fn reenqueue_pending(&self, event_ids: &[String], target_session: &str) -> Result<()> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        for id in event_ids {
            self.conn.execute(
                "UPDATE inbox SET state='pending', delivered_at=0
                 WHERE event_id=?1 AND target_session=?2",
                params![id, target],
            )?;
        }
        Ok(())
    }

    /// Completed inbound rows for a session whose delivery is newer than
    /// `since`, oldest-first. Powers statusline/integration peeks.
    pub fn recently_delivered_for_session(
        &self,
        target_session: &str,
        since: u64,
    ) -> Result<Vec<InboxRow>> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             WHERE target_session=?1
               AND state IN ('delivered', 'injected', 'echo_consumed')
               AND delivered_at>=?2
             ORDER BY created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target, since], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Mark successfully injected rows as awaiting user-prompt echo
    /// suppression. These rows are no longer pending for turn context delivery.
    pub fn mark_injected_for_echo(&self, event_ids: &[String], target_session: &str) -> Result<()> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        for id in event_ids {
            self.conn.execute(
                "UPDATE inbox SET state='injected'
                 WHERE event_id=?1 AND target_session=?2 AND state='delivered'",
                params![id, target],
            )?;
        }
        Ok(())
    }

    pub fn injected_for_session(&self, target_session: &str) -> Result<Vec<InboxRow>> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             WHERE target_session=?1 AND state='injected'
             ORDER BY delivered_at ASC, created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn consume_injected_echo(&self, event_ids: &[String], target_session: &str) -> Result<()> {
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        for id in event_ids {
            self.conn.execute(
                "UPDATE inbox SET state='echo_consumed'
                 WHERE event_id=?1 AND target_session=?2 AND state='injected'",
                params![id, target],
            )?;
        }
        Ok(())
    }
}
