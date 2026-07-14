//! Session selection for explicit native resume surfaces.

use super::sessions::{row_to_session, COLS};
use super::*;

impl Store {
    pub(crate) fn session_row(&self, pubkey: &str) -> Result<Option<Session>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM sessions WHERE pubkey=?1"),
                [pubkey],
                row_to_session,
            )
            .optional()?)
    }

    /// Recent sessions that have exactly one native resume locator.
    pub fn list_resumable_sessions(&self, limit: u32) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM sessions s
             WHERE EXISTS (
                 SELECT 1 FROM session_locators l
                 WHERE l.pubkey=s.pubkey AND l.locator_kind=?1
             )
             ORDER BY s.created_at DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![LOCATOR_NATIVE_RESUME, limit], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
