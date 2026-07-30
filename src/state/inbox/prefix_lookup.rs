use super::{row_to_inbox, COLS};
use crate::state::{InboxRow, Store};
use anyhow::Result;
use rusqlite::params;

impl Store {
    /// Inbound ledger rows whose event id starts with `prefix`, newest first.
    pub fn inbox_by_event_prefix(&self, prefix: &str) -> Result<Vec<InboxRow>> {
        let pattern = format!("{}%", prefix.replace(['%', '_'], ""));
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             LEFT JOIN messages ON messages.message_id=inbox.event_id
             WHERE inbox.event_id LIKE ?1
             ORDER BY inbox.created_at DESC, inbox.target_pubkey ASC"
        ))?;
        let rows = stmt.query_map(params![pattern], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// One inbound ledger row for an event prefix and exact target pubkey.
    pub fn inbox_by_event_prefix_and_target(
        &self,
        prefix: &str,
        target_pubkey: &str,
    ) -> Result<Vec<InboxRow>> {
        let pattern = format!("{}%", prefix.replace(['%', '_'], ""));
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             LEFT JOIN messages ON messages.message_id=inbox.event_id
             WHERE inbox.event_id LIKE ?1 AND inbox.target_pubkey=?2
             ORDER BY inbox.created_at DESC"
        ))?;
        let rows = stmt.query_map(params![pattern, target_pubkey], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
