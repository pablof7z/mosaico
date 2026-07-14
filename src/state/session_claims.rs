use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionClaim {
    pub(crate) pubkey: String,
    pub(crate) agent_slug: String,
    pub(crate) channel_h: String,
    pub(crate) harness: String,
    pub(crate) last_active_at: u64,
    pub(crate) expires_at: u64,
    pub(crate) owner_backend_pubkey: String,
    pub(crate) owner_host: String,
}

const COLS: &str = "pubkey, agent_slug, channel_h, harness, last_active_at, expires_at, \
                     owner_backend_pubkey, owner_host";

impl SessionClaim {
    pub(crate) fn is_owned_by_backend(&self, backend_pubkey: Option<&str>) -> bool {
        self.owner_backend_pubkey.is_empty()
            || backend_pubkey
                .filter(|pubkey| !pubkey.is_empty())
                .is_some_and(|pubkey| pubkey == self.owner_backend_pubkey)
    }
}

fn row_to_claim(row: &rusqlite::Row) -> rusqlite::Result<SessionClaim> {
    Ok(SessionClaim {
        pubkey: row.get(0)?,
        agent_slug: row.get(1)?,
        channel_h: row.get(2)?,
        harness: row.get(3)?,
        last_active_at: row.get(4)?,
        expires_at: row.get(5)?,
        owner_backend_pubkey: row.get(6)?,
        owner_host: row.get(7)?,
    })
}

impl Store {
    pub(crate) fn upsert_session_claim(&self, claim: &SessionClaim) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_claims
                 (pubkey, agent_slug, channel_h, harness, last_active_at, expires_at,
                  owner_backend_pubkey, owner_host)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(pubkey, channel_h) DO UPDATE SET
                 agent_slug=excluded.agent_slug, harness=excluded.harness,
                 last_active_at=excluded.last_active_at, expires_at=excluded.expires_at,
                 owner_backend_pubkey=excluded.owner_backend_pubkey,
                 owner_host=excluded.owner_host",
            params![
                claim.pubkey,
                claim.agent_slug,
                claim.channel_h,
                claim.harness,
                claim.last_active_at,
                claim.expires_at,
                claim.owner_backend_pubkey,
                claim.owner_host,
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
            "SELECT {COLS} FROM session_claims
             WHERE expires_at>=?1 ORDER BY last_active_at DESC"
        ))?;
        let rows = stmt.query_map([now], row_to_claim)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn clear_session_claim_for_pubkey(&self, pubkey: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM session_claims WHERE pubkey=?1", [pubkey])?;
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
}

#[cfg(test)]
#[path = "session_claims/tests.rs"]
mod tests;
