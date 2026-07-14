//! Pubkey-owned reconstruction material for ordinary session signers.

use super::*;

impl Store {
    #[cfg(test)]
    pub(crate) fn bind_session_signer(&self, pubkey: &str, signer_salt: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO session_signers (pubkey, signer_salt) VALUES (?1, ?2)",
            params![pubkey, signer_salt],
        )?;
        let stored = self
            .session_signer_salt(pubkey)?
            .context("inserted session signer is missing")?;
        if stored != signer_salt {
            anyhow::bail!("signer material changed for pubkey {pubkey:?}");
        }
        Ok(())
    }

    pub(crate) fn session_signer_salt(&self, pubkey: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT signer_salt FROM session_signers WHERE pubkey=?1",
                [pubkey],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn is_derived_session_pubkey(&self, pubkey: &str) -> Result<bool> {
        Ok(self.session_signer_salt(pubkey)?.is_some())
    }

    pub(crate) fn list_local_session_pubkeys(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT pubkey FROM session_signers
             UNION SELECT pubkey FROM sessions
             UNION SELECT pubkey FROM session_claims
             ORDER BY pubkey",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signer_material_is_immutable_and_owned_by_pubkey() {
        let store = Store::open_memory().unwrap();
        store.bind_session_signer("pk", "salt-a").unwrap();
        store.bind_session_signer("pk", "salt-a").unwrap();
        let error = store.bind_session_signer("pk", "salt-b").unwrap_err();
        assert!(error.to_string().contains("signer material changed"));
        assert_eq!(
            store.session_signer_salt("pk").unwrap().as_deref(),
            Some("salt-a")
        );
    }
}
