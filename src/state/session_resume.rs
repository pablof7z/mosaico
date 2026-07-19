//! Session selection for explicit native resume surfaces.

use super::sessions::{row_to_session, COLS};
use super::*;

impl Store {
    /// Every durable local session, newest activity first.
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM sessions
             ORDER BY MAX(last_seen, stopped_at, created_at) DESC, pubkey ASC"
        ))?;
        let rows = stmt.query_map([], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
