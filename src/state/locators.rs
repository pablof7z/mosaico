//! Typed host-local runtime locators. Locators are never identities: every row
//! resolves directly to the session pubkey.

use super::*;
use rusqlite::{Transaction, TransactionBehavior};

pub(crate) const LOCATOR_NATIVE_RESUME: &str = "native_resume";
pub(crate) const LOCATOR_PTY: &str = "pty";
pub(crate) const LOCATOR_ACP: &str = "acp";
pub(crate) const LOCATOR_APP_SERVER: &str = "app_server";
pub(crate) const LOCATOR_PID: &str = "pid";

const COLS: &str = "harness, locator_kind, locator_value, pubkey, runtime_generation, created_at";

fn row_to_locator(row: &rusqlite::Row) -> rusqlite::Result<SessionLocator> {
    Ok(SessionLocator {
        harness: row.get(0)?,
        locator_kind: row.get(1)?,
        locator_value: row.get(2)?,
        pubkey: row.get(3)?,
        runtime_generation: row.get(4)?,
        created_at: row.get(5)?,
    })
}

impl Store {
    pub fn put_session_locator(
        &self,
        harness: &str,
        locator_kind: &str,
        locator_value: &str,
        pubkey: &str,
        created_at: u64,
    ) -> Result<()> {
        validate_locator_kind(locator_kind)?;
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let (recovery, generation) = tx
            .query_row(
                "SELECT recovery_state, runtime_generation FROM sessions WHERE pubkey=?1",
                [pubkey],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .optional()?
            .context("session locator has no session")?;
        let adds_delivery_path =
            matches!(locator_kind, LOCATOR_PTY | LOCATOR_ACP | LOCATOR_APP_SERVER)
                && !tx.query_row(
                    "SELECT EXISTS(
                SELECT 1 FROM session_locators
                 WHERE pubkey=?1 AND harness=?2 AND locator_kind=?3
                   AND runtime_generation=?4
            )",
                    params![pubkey, harness, locator_kind, generation],
                    |row| row.get::<_, bool>(0),
                )?;
        if locator_kind == LOCATOR_NATIVE_RESUME {
            if recovery == RecoveryState::Revoked.as_str() {
                anyhow::bail!("pubkey {pubkey} recovery authority is revoked");
            }
            tx.execute(
                "DELETE FROM session_locators WHERE pubkey=?1 AND locator_kind=?2",
                params![pubkey, LOCATOR_NATIVE_RESUME],
            )?;
        } else {
            tx.execute(
                "DELETE FROM session_locators
                 WHERE pubkey=?1 AND harness=?2 AND locator_kind=?3",
                params![pubkey, harness, locator_kind],
            )?;
        }
        if adds_delivery_path {
            tx.execute(
                "UPDATE sessions SET state_changed_at=?3
                 WHERE pubkey=?1 AND runtime_generation=?2
                   AND runtime_state='running' AND work_state='idle'",
                params![pubkey, generation, created_at],
            )?;
        }
        let locator_generation = if locator_kind == LOCATOR_NATIVE_RESUME {
            0
        } else {
            generation
        };
        tx.execute(
            "INSERT INTO session_locators
                 (harness, locator_kind, locator_value, pubkey, runtime_generation, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(harness, locator_kind, locator_value)
             DO UPDATE SET pubkey=excluded.pubkey,
                           runtime_generation=excluded.runtime_generation,
                           created_at=excluded.created_at",
            params![
                harness,
                locator_kind,
                locator_value,
                pubkey,
                locator_generation,
                created_at
            ],
        )?;
        if locator_kind == LOCATOR_NATIVE_RESUME {
            tx.execute(
                "UPDATE sessions SET recovery_state='ready' WHERE pubkey=?1",
                [pubkey],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn resolve_pubkey_by_locator(
        &self,
        harness: &str,
        locator_kind: &str,
        locator_value: &str,
    ) -> Result<Option<String>> {
        validate_locator_kind(locator_kind)?;
        Ok(self
            .conn
            .query_row(
                "SELECT pubkey FROM session_locators
             WHERE harness=?1 AND locator_kind=?2 AND locator_value=?3",
                params![harness, locator_kind, locator_value],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn running_session_for_locator(
        &self,
        harness: Option<&str>,
        locator_kind: &str,
        locator_value: &str,
    ) -> Result<Option<Session>> {
        validate_locator_kind(locator_kind)?;
        let pubkey: Option<String> = match harness {
            Some(harness) => self
                .conn
                .query_row(
                    "SELECT l.pubkey FROM session_locators l
                 JOIN sessions s ON s.pubkey=l.pubkey
                 WHERE l.harness=?1 AND l.locator_kind=?2 AND l.locator_value=?3
                   AND s.runtime_state='running'
                   AND (l.runtime_generation=0 OR l.runtime_generation=s.runtime_generation)
                 LIMIT 1",
                    params![harness, locator_kind, locator_value],
                    |row| row.get::<_, String>(0),
                )
                .optional()?,
            None => self
                .conn
                .query_row(
                    "SELECT l.pubkey FROM session_locators l
                 JOIN sessions s ON s.pubkey=l.pubkey
                 WHERE l.locator_kind=?1 AND l.locator_value=?2
                   AND s.runtime_state='running'
                   AND (l.runtime_generation=0 OR l.runtime_generation=s.runtime_generation)
                 ORDER BY l.created_at DESC LIMIT 1",
                    params![locator_kind, locator_value],
                    |row| row.get::<_, String>(0),
                )
                .optional()?,
        };
        match pubkey {
            Some(pubkey) => self.get_session(&pubkey),
            None => Ok(None),
        }
    }

    pub fn session_for_runtime_locator(
        &self,
        locator_kind: &str,
        locator_value: &str,
    ) -> Result<Option<Session>> {
        validate_locator_kind(locator_kind)?;
        let row = self
            .conn
            .query_row(
                "SELECT l.pubkey, l.runtime_generation
                 FROM session_locators l
                 WHERE l.locator_kind=?1 AND l.locator_value=?2
                   AND l.runtime_generation>0
                 ORDER BY l.created_at DESC LIMIT 1",
                params![locator_kind, locator_value],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .optional()?;
        let Some((pubkey, generation)) = row else {
            return Ok(None);
        };
        Ok(self
            .get_session(&pubkey)?
            .filter(|session| session.runtime_generation == generation))
    }

    pub fn locators_for_value(
        &self,
        harness: Option<&str>,
        locator_kind: &str,
        locator_value: &str,
    ) -> Result<Vec<SessionLocator>> {
        validate_locator_kind(locator_kind)?;
        let sql = match harness {
            Some(_) => format!(
                "SELECT {COLS} FROM session_locators
                 WHERE harness=?1 AND locator_kind=?2 AND locator_value=?3
                 ORDER BY created_at DESC"
            ),
            None => format!(
                "SELECT {COLS} FROM session_locators
                 WHERE locator_kind=?1 AND locator_value=?2 ORDER BY created_at DESC"
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = match harness {
            Some(harness) => stmt.query_map(
                params![harness, locator_kind, locator_value],
                row_to_locator,
            )?,
            None => stmt.query_map(params![locator_kind, locator_value], row_to_locator)?,
        };
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_locators_of_kind(&self, locator_kind: &str) -> Result<Vec<SessionLocator>> {
        validate_locator_kind(locator_kind)?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM session_locators
             WHERE locator_kind=?1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt.query_map([locator_kind], row_to_locator)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn locators_for_pubkey(&self, pubkey: &str) -> Result<Vec<SessionLocator>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM session_locators WHERE pubkey=?1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt.query_map([pubkey], row_to_locator)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn runtime_locator_for_session(
        &self,
        pubkey: &str,
        runtime_generation: u64,
        locator_kind: &str,
    ) -> Result<Option<SessionLocator>> {
        validate_locator_kind(locator_kind)?;
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM session_locators
                     WHERE pubkey=?1 AND runtime_generation=?2 AND locator_kind=?3
                       AND harness=(SELECT observed_harness FROM sessions WHERE pubkey=?1)"
                ),
                params![pubkey, runtime_generation, locator_kind],
                row_to_locator,
            )
            .optional()?)
    }

    pub fn locator_for_session(
        &self,
        pubkey: &str,
        harness: &str,
        locator_kind: &str,
    ) -> Result<Option<SessionLocator>> {
        validate_locator_kind(locator_kind)?;
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM session_locators
                     WHERE pubkey=?1 AND harness=?2 AND locator_kind=?3
                     ORDER BY created_at DESC LIMIT 1"
                ),
                params![pubkey, harness, locator_kind],
                row_to_locator,
            )
            .optional()?)
    }

    pub fn native_resume_locator(
        &self,
        pubkey: &str,
        harness: &str,
    ) -> Result<Option<SessionLocator>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM session_locators
                 WHERE pubkey=?1 AND harness=?2 AND locator_kind=?3"
                ),
                params![pubkey, harness, LOCATOR_NATIVE_RESUME],
                row_to_locator,
            )
            .optional()?)
    }

    pub fn clear_locator_kind(&self, pubkey: &str, locator_kind: &str) -> Result<()> {
        validate_locator_kind(locator_kind)?;
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM session_locators WHERE pubkey=?1 AND locator_kind=?2",
            params![pubkey, locator_kind],
        )?;
        if locator_kind == LOCATOR_NATIVE_RESUME {
            transaction.execute(
                "UPDATE sessions SET recovery_state='pending'
                 WHERE pubkey=?1 AND recovery_state='ready'",
                [pubkey],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn clear_runtime_locator_if_generation(
        &self,
        pubkey: &str,
        locator_kind: &str,
        runtime_generation: u64,
    ) -> Result<bool> {
        validate_locator_kind(locator_kind)?;
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let removed = transaction.execute(
            "DELETE FROM session_locators
             WHERE pubkey=?1 AND locator_kind=?2 AND runtime_generation=?3
               AND harness=(SELECT observed_harness FROM sessions WHERE pubkey=?1)",
            params![pubkey, locator_kind, runtime_generation],
        )? > 0;
        if removed && matches!(locator_kind, LOCATOR_PTY | LOCATOR_ACP | LOCATOR_APP_SERVER) {
            transaction.execute(
                "UPDATE sessions SET state_changed_at=?3
                 WHERE pubkey=?1 AND runtime_generation=?2
                   AND runtime_state='running' AND work_state='idle'",
                params![pubkey, runtime_generation, crate::util::now_secs()],
            )?;
        }
        transaction.commit()?;
        Ok(removed)
    }

    pub fn clear_session_locator_kind(
        &self,
        pubkey: &str,
        harness: &str,
        locator_kind: &str,
    ) -> Result<()> {
        validate_locator_kind(locator_kind)?;
        self.conn.execute(
            "DELETE FROM session_locators
             WHERE pubkey=?1 AND harness=?2 AND locator_kind=?3",
            params![pubkey, harness, locator_kind],
        )?;
        Ok(())
    }
}

fn validate_locator_kind(locator_kind: &str) -> Result<()> {
    match locator_kind {
        LOCATOR_NATIVE_RESUME | LOCATOR_PTY | LOCATOR_ACP | LOCATOR_APP_SERVER | LOCATOR_PID => {
            Ok(())
        }
        _ => anyhow::bail!("unknown session locator kind {locator_kind:?}"),
    }
}

#[cfg(test)]
#[path = "locators/tests.rs"]
mod tests;
