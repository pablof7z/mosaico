use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionClaim {
    pub(crate) pubkey: String,
    pub(crate) base_pubkey: String,
    pub(crate) agent_slug: String,
    pub(crate) ordinal: u32,
    pub(crate) session_id: String,
    pub(crate) channel_h: String,
    pub(crate) native_id: String,
    pub(crate) harness: String,
    pub(crate) last_active_at: u64,
    pub(crate) expires_at: u64,
}

const COLS: &str = "pubkey, base_pubkey, agent_slug, ordinal, session_id, channel_h, native_id, \
     harness, last_active_at, expires_at";

fn row_to_claim(row: &rusqlite::Row) -> rusqlite::Result<SessionClaim> {
    Ok(SessionClaim {
        pubkey: row.get(0)?,
        base_pubkey: row.get(1)?,
        agent_slug: row.get(2)?,
        ordinal: row.get::<_, i64>(3)? as u32,
        session_id: row.get(4)?,
        channel_h: row.get(5)?,
        native_id: row.get(6)?,
        harness: row.get(7)?,
        last_active_at: row.get(8)?,
        expires_at: row.get(9)?,
    })
}

impl Store {
    pub(crate) fn upsert_session_claim(&self, c: &SessionClaim) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_claims
                 (pubkey, base_pubkey, agent_slug, ordinal, session_id, channel_h, native_id,
                  harness, last_active_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(pubkey, channel_h) DO UPDATE SET
                 base_pubkey=excluded.base_pubkey, agent_slug=excluded.agent_slug,
                 ordinal=excluded.ordinal, session_id=excluded.session_id,
                 native_id=excluded.native_id, harness=excluded.harness,
                 last_active_at=excluded.last_active_at, expires_at=excluded.expires_at",
            params![
                c.pubkey,
                c.base_pubkey,
                c.agent_slug,
                c.ordinal as i64,
                c.session_id,
                c.channel_h,
                c.native_id,
                c.harness,
                c.last_active_at,
                c.expires_at
            ],
        )?;
        Ok(())
    }

    pub(crate) fn get_session_claim(
        &self,
        pubkey: &str,
        channel_h: &str,
    ) -> Result<Option<SessionClaim>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM session_claims WHERE pubkey=?1 AND channel_h=?2"),
                params![pubkey, channel_h],
                row_to_claim,
            )
            .optional()?)
    }

    pub(crate) fn get_active_session_claim(
        &self,
        pubkey: &str,
        channel_h: &str,
        now: u64,
    ) -> Result<Option<SessionClaim>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM session_claims
                     WHERE pubkey=?1 AND channel_h=?2 AND expires_at>=?3"
                ),
                params![pubkey, channel_h, now],
                row_to_claim,
            )
            .optional()?)
    }

    pub(crate) fn list_active_session_claims(&self, now: u64) -> Result<Vec<SessionClaim>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM session_claims WHERE expires_at>=?1 ORDER BY last_active_at DESC"
        ))?;
        let rows = stmt.query_map(params![now], row_to_claim)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn list_active_session_claims_for_channel(
        &self,
        channel_h: &str,
        now: u64,
    ) -> Result<Vec<SessionClaim>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM session_claims
             WHERE channel_h=?1 AND expires_at>=?2 ORDER BY last_active_at DESC"
        ))?;
        let rows = stmt.query_map(params![channel_h, now], row_to_claim)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn clear_session_claim_for_session(&self, session_id: &str) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(session_id)? else {
            return Ok(());
        };
        self.conn.execute(
            "DELETE FROM session_claims WHERE session_id=?1",
            params![canonical],
        )?;
        Ok(())
    }

    pub(crate) fn clear_session_claim_for_route(
        &self,
        pubkey: &str,
        channel_h: &str,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM session_claims WHERE pubkey=?1 AND channel_h=?2",
            params![pubkey, channel_h],
        )?;
        Ok(())
    }

    pub(crate) fn clear_session_claims_for_reassert(
        &self,
        session_id: &str,
        pubkey: &str,
        channel_h: &str,
    ) -> Result<()> {
        self.clear_session_claim_for_session(session_id)?;
        self.clear_session_claim_for_route(pubkey, channel_h)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "session_claims/tests.rs"]
mod tests;
