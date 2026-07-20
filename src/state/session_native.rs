use super::*;
use rusqlite::{Transaction, TransactionBehavior};

impl Store {
    /// Bind or rotate the one harness-native resume locator for a pubkey.
    pub fn set_native_resume_locator(
        &self,
        pubkey: &str,
        harness: &str,
        native_resume: &str,
        now: u64,
    ) -> Result<()> {
        if !self.session_exists(pubkey)? {
            anyhow::bail!("cannot bind native resume locator for unknown pubkey {pubkey}");
        }
        self.put_session_locator(harness, LOCATOR_NATIVE_RESUME, native_resume, pubkey, now)
    }

    /// Claim an unowned native locator without ever stealing an existing
    /// mapping. Returns the authoritative owner, which is `pubkey` when this
    /// call won the claim. The immediate transaction makes concurrent adoption
    /// attempts converge on one session identity.
    pub fn claim_native_resume_locator(
        &self,
        pubkey: &str,
        harness: &str,
        native_resume: &str,
        now: u64,
    ) -> Result<String> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let recovery = tx
            .query_row(
                "SELECT recovery_state FROM sessions WHERE pubkey=?1",
                [pubkey],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .with_context(|| format!("cannot claim native locator for unknown pubkey {pubkey}"))?;
        if recovery == RecoveryState::Revoked.as_str() {
            anyhow::bail!("pubkey {pubkey} recovery authority is revoked");
        }
        if let Some(owner) = tx
            .query_row(
                "SELECT pubkey FROM session_locators
                 WHERE harness=?1 AND locator_kind=?2 AND locator_value=?3",
                params![harness, LOCATOR_NATIVE_RESUME, native_resume],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            tx.commit()?;
            return Ok(owner);
        }
        anyhow::ensure!(
            tx.query_row(
                "SELECT 1 FROM session_locators
                 WHERE pubkey=?1 AND locator_kind=?2 LIMIT 1",
                params![pubkey, LOCATOR_NATIVE_RESUME],
                |_| Ok(()),
            )
            .optional()?
            .is_none(),
            "pubkey {pubkey} already owns a native resume locator"
        );
        tx.execute(
            "INSERT INTO session_locators
                 (harness, locator_kind, locator_value, pubkey, runtime_generation, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![harness, LOCATOR_NATIVE_RESUME, native_resume, pubkey, now],
        )?;
        tx.execute(
            "UPDATE sessions SET recovery_state='ready' WHERE pubkey=?1",
            [pubkey],
        )?;
        tx.commit()?;
        Ok(pubkey.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(store: &Store, pubkey: &str) {
        store
            .reserve_hook_session_for_test(&RegisterSession {
                pubkey: pubkey.into(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                channel_h: "mosaico".into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            })
            .unwrap();
    }

    #[test]
    fn native_locator_claim_never_steals_the_winner() {
        let store = Store::open_memory().unwrap();
        register(&store, "pk-one");
        register(&store, "pk-two");

        assert_eq!(
            store
                .claim_native_resume_locator("pk-one", "codex", "native-1", 2)
                .unwrap(),
            "pk-one"
        );
        assert_eq!(
            store
                .claim_native_resume_locator("pk-two", "codex", "native-1", 3)
                .unwrap(),
            "pk-one"
        );
    }
}
