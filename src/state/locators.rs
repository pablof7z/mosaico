//! Typed host-local runtime locators. Locators are never identities: every row
//! resolves directly to the session pubkey.

use super::*;
use rusqlite::{Transaction, TransactionBehavior};

pub(crate) const LOCATOR_NATIVE_RESUME: &str = "native_resume";
pub(crate) const LOCATOR_PTY: &str = "pty";
pub(crate) const LOCATOR_ACP: &str = "acp";
pub(crate) const LOCATOR_APP_SERVER: &str = "app_server";
pub(crate) const LOCATOR_PID: &str = "pid";

const COLS: &str = "harness, locator_kind, locator_value, pubkey, created_at";

fn row_to_locator(row: &rusqlite::Row) -> rusqlite::Result<SessionLocator> {
    Ok(SessionLocator {
        harness: row.get(0)?,
        locator_kind: row.get(1)?,
        locator_value: row.get(2)?,
        pubkey: row.get(3)?,
        created_at: row.get(4)?,
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
        if locator_kind == LOCATOR_NATIVE_RESUME {
            tx.execute(
                "DELETE FROM session_locators WHERE pubkey=?1 AND locator_kind=?2",
                params![pubkey, LOCATOR_NATIVE_RESUME],
            )?;
        }
        tx.execute(
            "INSERT INTO session_locators
                 (harness, locator_kind, locator_value, pubkey, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(harness, locator_kind, locator_value)
             DO UPDATE SET pubkey=excluded.pubkey, created_at=excluded.created_at",
            params![harness, locator_kind, locator_value, pubkey, created_at],
        )?;
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

    pub fn alive_session_for_locator(
        &self,
        harness: &str,
        locator_kind: &str,
        locator_value: &str,
    ) -> Result<Option<Session>> {
        validate_locator_kind(locator_kind)?;
        let pubkey: Option<String> = self
            .conn
            .query_row(
                "SELECT l.pubkey FROM session_locators l
                 JOIN sessions s ON s.pubkey=l.pubkey
                 WHERE l.harness=?1 AND l.locator_kind=?2 AND l.locator_value=?3
                   AND s.alive=1 LIMIT 1",
                params![harness, locator_kind, locator_value],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        match pubkey {
            Some(pubkey) => self.get_session(&pubkey),
            None => Ok(None),
        }
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
        self.conn.execute(
            "DELETE FROM session_locators WHERE pubkey=?1 AND locator_kind=?2",
            params![pubkey, locator_kind],
        )?;
        Ok(())
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

    pub fn retire_dead_endpoint(&self, pubkey: &str) -> Result<()> {
        self.clear_locator_kind(pubkey, LOCATOR_PTY)?;
        self.mark_dead(pubkey)
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
mod tests {
    use super::*;

    fn registration(pubkey: &str, at: u64) -> RegisterSession {
        RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            channel_h: "root".into(),
            child_pid: None,
            transcript_path: None,
            now: at,
        }
    }

    #[test]
    fn native_resume_is_stored_once_per_pubkey() {
        let store = Store::open_memory().unwrap();
        store
            .reserve_hook_session_for_test(&registration("pk", 1))
            .unwrap();
        store
            .put_session_locator("codex", LOCATOR_NATIVE_RESUME, "old", "pk", 2)
            .unwrap();
        store
            .put_session_locator("codex", LOCATOR_NATIVE_RESUME, "new", "pk", 3)
            .unwrap();

        let locator = store.native_resume_locator("pk", "codex").unwrap().unwrap();
        assert_eq!(locator.locator_value, "new");
        assert!(store
            .native_resume_locator("pk", "claude-code")
            .unwrap()
            .is_none());
        assert!(store
            .resolve_pubkey_by_locator("codex", LOCATOR_NATIVE_RESUME, "old")
            .unwrap()
            .is_none());
    }

    #[test]
    fn locator_vocabulary_is_closed() {
        let store = Store::open_memory().unwrap();
        store
            .reserve_hook_session_for_test(&registration("pk", 1))
            .unwrap();
        let error = store
            .put_session_locator("codex", "harness_session", "old", "pk", 2)
            .unwrap_err();
        assert!(error.to_string().contains("unknown session locator kind"));
    }

    #[test]
    fn session_locator_lookup_requires_the_observed_harness_dimension() {
        let store = Store::open_memory().unwrap();
        store
            .reserve_hook_session_for_test(&registration("pk", 1))
            .unwrap();
        store
            .put_session_locator("claude-code", LOCATOR_PTY, "foreign", "pk", 3)
            .unwrap();
        store
            .put_session_locator("codex", LOCATOR_PTY, "owned", "pk", 2)
            .unwrap();

        assert_eq!(
            store
                .locator_for_session("pk", "codex", LOCATOR_PTY)
                .unwrap()
                .unwrap()
                .locator_value,
            "owned"
        );
        assert_eq!(
            store
                .locator_for_session("pk", "claude-code", LOCATOR_PTY)
                .unwrap()
                .unwrap()
                .locator_value,
            "foreign"
        );
    }
}
