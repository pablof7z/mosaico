//! Session selection for explicit resume surfaces.

use super::sessions::{row_to_session, COLS};
use super::*;

impl Store {
    /// Durable-agent sessions never resume a dead historical session. Reassert
    /// the current live alias owner, otherwise mint and repoint to a fresh id.
    pub(crate) fn resolve_live_or_mint_session_id(
        &self,
        harness: &str,
        external_id_kind: &str,
        external_id: &str,
        now: u64,
    ) -> Result<String> {
        let id = self
            .resolve_session_by_alias(harness, external_id_kind, external_id)?
            .and_then(|id| {
                self.get_session(&id)
                    .ok()
                    .flatten()
                    .filter(|session| session.alive)
                    .map(|_| id)
            })
            .unwrap_or_else(mint_session_id);
        self.put_alias(harness, external_id_kind, external_id, &id, now)?;
        Ok(id)
    }

    /// Recent resumable per-session identities, newest first. Durable agents
    /// always start fresh and therefore never appear in this candidate set.
    pub fn list_resumable_sessions(&self, limit: u32) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM sessions
             WHERE NOT EXISTS (
                 SELECT 1 FROM durable_agent_sessions d
                 WHERE d.pubkey=sessions.agent_pubkey
             )
             ORDER BY created_at DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![limit], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
