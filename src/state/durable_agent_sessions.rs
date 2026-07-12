use super::*;
use rusqlite::{Transaction, TransactionBehavior};

impl Store {
    pub(crate) fn claim_durable_agent_session(
        &self,
        pubkey: &str,
        agent_slug: &str,
        session_id: &str,
        now: u64,
    ) -> Result<bool> {
        self.claim_durable_agent_session_with_reservation(pubkey, agent_slug, session_id, None, now)
    }

    pub(crate) fn claim_durable_agent_session_with_reservation(
        &self,
        pubkey: &str,
        agent_slug: &str,
        session_id: &str,
        reservation_id: Option<&str>,
        now: u64,
    ) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let other_live_session = tx
            .query_row(
                "SELECT session_id FROM sessions
                 WHERE alive=1 AND agent_slug=?1 AND session_id<>?2
                 ORDER BY created_at DESC LIMIT 1",
                params![agent_slug, session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if other_live_session.is_some() {
            anyhow::bail!(
                "durable agent {agent_slug:?} already has a live session on this backend"
            );
        }
        let existing = tx
            .query_row(
                "SELECT pubkey, agent_slug, session_id, live
                 FROM durable_agent_sessions
                 WHERE pubkey=?1 OR agent_slug=?2",
                params![pubkey, agent_slug],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        let reasserted = if let Some((owner_pubkey, owner_slug, owner_session, live)) = existing {
            if owner_pubkey != pubkey || owner_slug != agent_slug {
                anyhow::bail!(
                    "durable agent identity collision for {agent_slug:?}; configured key belongs to another agent"
                );
            }
            let owns_reservation = reservation_id == Some(owner_session.as_str());
            if live && owner_session != session_id && !owns_reservation {
                anyhow::bail!(
                    "durable agent {agent_slug:?} already has a live session on this backend"
                );
            }
            live && owner_session == session_id
        } else {
            false
        };
        tx.execute(
            "INSERT INTO durable_agent_sessions
                 (pubkey, agent_slug, session_id, live, updated_at)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(pubkey) DO UPDATE SET
                 agent_slug=excluded.agent_slug,
                 session_id=excluded.session_id,
                 live=1,
                 updated_at=excluded.updated_at",
            params![pubkey, agent_slug, session_id, now],
        )?;
        tx.commit()?;
        Ok(!reasserted)
    }

    pub(crate) fn release_durable_agent_session(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE durable_agent_sessions SET live=0 WHERE session_id=?1",
            [session_id],
        )?;
        Ok(())
    }

    pub(crate) fn is_durable_agent_pubkey(&self, pubkey: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_agent_sessions WHERE pubkey=?1)",
            [pubkey],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn is_durable_agent_session(&self, session_id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_agent_sessions WHERE session_id=?1)",
            [session_id],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn live_durable_session_for_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_id FROM durable_agent_sessions
                 WHERE pubkey=?1 AND live=1",
                [pubkey],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn cleanup_orphan_durable_sessions(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE durable_agent_sessions SET live=0
             WHERE live=1 AND NOT EXISTS (
                 SELECT 1 FROM sessions s
                 WHERE s.session_id=durable_agent_sessions.session_id
                   AND s.agent_pubkey=durable_agent_sessions.pubkey
                   AND s.agent_slug=durable_agent_sessions.agent_slug
                   AND s.alive=1
             )",
            [],
        )?)
    }
}

#[cfg(test)]
#[path = "durable_agent_sessions/tests.rs"]
mod tests;
