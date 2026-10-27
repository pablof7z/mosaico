//! Durable retirement work for reclaimed session handles.
//!
//! When a new session reclaims a handle, the previous owner's kind:0 still
//! advertises it until a retirement profile reaches the relay. The work is
//! recorded before any publication is attempted and cleared only once the
//! relay acknowledges it, so relay failures and daemon restarts retry it.

use super::*;

/// First retry after a failure, doubling per failure up to the cap.
const RETRY_BASE_SECS: u64 = 5;
const RETRY_CAP_SECS: u64 = 600;

pub(crate) fn retry_delay_secs(failures: u32) -> u64 {
    RETRY_BASE_SECS
        .saturating_mul(1_u64 << failures.saturating_sub(1).min(16))
        .min(RETRY_CAP_SECS)
}

impl Store {
    pub(crate) fn schedule_profile_retirement(&self, pubkey: &str, now: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO profile_retirements (pubkey, due_at, attempts) VALUES (?1, ?2, 0)
             ON CONFLICT(pubkey) DO NOTHING",
            params![pubkey, now],
        )?;
        Ok(())
    }

    /// Retirements whose next attempt is due at `now`.
    pub(crate) fn list_due_profile_retirements(&self, now: u64) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            "SELECT pubkey FROM profile_retirements WHERE due_at <= ?1 ORDER BY due_at, pubkey",
        )?;
        let rows = statement.query_map([now], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Record a failed attempt and back the next one off. Returns how many
    /// attempts have now failed.
    pub(crate) fn record_profile_retirement_failure(&self, pubkey: &str, now: u64) -> Result<u32> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let failures: u32 = transaction.query_row(
            "UPDATE profile_retirements SET attempts=attempts+1 WHERE pubkey=?1
             RETURNING attempts",
            [pubkey],
            |row| row.get(0),
        )?;
        transaction.execute(
            "UPDATE profile_retirements SET due_at=?2 WHERE pubkey=?1",
            params![pubkey, now.saturating_add(retry_delay_secs(failures))],
        )?;
        transaction.commit()?;
        Ok(failures)
    }

    /// Make every retirement due now with a fresh backoff: the relay changed,
    /// so earlier failures say nothing about the next attempt.
    pub(crate) fn reset_profile_retirement_backoff(&self) -> Result<()> {
        self.conn
            .execute("UPDATE profile_retirements SET due_at=0, attempts=0", [])?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn profile_retirement_attempts(&self, pubkey: &str) -> Result<Option<u32>> {
        Ok(self
            .conn
            .query_row(
                "SELECT attempts FROM profile_retirements WHERE pubkey=?1",
                [pubkey],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn complete_profile_retirement(&self, pubkey: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM profile_retirements WHERE pubkey=?1", [pubkey])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_retirement_backs_off_exponentially_up_to_the_cap() {
        let store = Store::open_memory().unwrap();
        store.schedule_profile_retirement("pk", 100).unwrap();
        assert_eq!(store.list_due_profile_retirements(100).unwrap(), ["pk"]);

        assert_eq!(
            store.record_profile_retirement_failure("pk", 100).unwrap(),
            1
        );
        assert!(store.list_due_profile_retirements(104).unwrap().is_empty());
        assert_eq!(store.list_due_profile_retirements(105).unwrap(), ["pk"]);

        assert_eq!(
            store.record_profile_retirement_failure("pk", 105).unwrap(),
            2
        );
        assert!(store.list_due_profile_retirements(114).unwrap().is_empty());
        assert_eq!(store.list_due_profile_retirements(115).unwrap(), ["pk"]);

        let delays = (1..=10).map(retry_delay_secs).collect::<Vec<_>>();
        assert_eq!(delays, [5, 10, 20, 40, 80, 160, 320, 600, 600, 600]);

        store.reset_profile_retirement_backoff().unwrap();
        assert_eq!(store.list_due_profile_retirements(0).unwrap(), ["pk"]);
        assert_eq!(store.profile_retirement_attempts("pk").unwrap(), Some(0));

        store.complete_profile_retirement("pk").unwrap();
        assert!(store
            .list_due_profile_retirements(i64::MAX as u64)
            .unwrap()
            .is_empty());
    }
}
