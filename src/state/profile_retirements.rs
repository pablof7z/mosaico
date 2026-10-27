//! Durable retirement work for reclaimed session handles.
//!
//! When a new session reclaims a handle, the previous owner's kind:0 still
//! advertises it until a retirement profile reaches the relay. The work is
//! recorded before any publication is attempted and cleared only once the
//! relay acknowledges it, so relay failures and daemon restarts retry it.

use super::*;

impl Store {
    pub(crate) fn schedule_profile_retirement(&self, pubkey: &str, now: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO profile_retirements (pubkey, due_at) VALUES (?1, ?2)
             ON CONFLICT(pubkey) DO NOTHING",
            params![pubkey, now],
        )?;
        Ok(())
    }

    pub(crate) fn list_due_profile_retirements(&self) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT pubkey FROM profile_retirements ORDER BY due_at, pubkey")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn complete_profile_retirement(&self, pubkey: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM profile_retirements WHERE pubkey=?1", [pubkey])?;
        Ok(())
    }
}
