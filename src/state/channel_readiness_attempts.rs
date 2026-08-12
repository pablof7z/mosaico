//! `channel_readiness_attempts` records host/provider readiness decisions.
//!
//! NMP's delivered group state remains the source of channel truth. These rows
//! only explain local attempts to make that truth exist or become usable.

use super::*;

const MAX_RETAINED_ATTEMPTS: i64 = 500;
const RETAIN_FOR_SECS: u64 = 60 * 60;

/// Durable record of a host/provider channel readiness attempt. These are not
/// authoritative channel state; they explain local provisioning decisions that
/// otherwise only existed in daemon logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelReadinessAttempt {
    pub id: i64,
    pub channel_h: String,
    pub expect_member: String,
    pub parent_hint: Option<String>,
    pub name: Option<String>,
    pub source: String,
    pub outcome: String,
    pub reason: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChannelReadinessAttempt {
    pub channel_h: String,
    pub expect_member: String,
    pub parent_hint: Option<String>,
    pub name: Option<String>,
    pub source: String,
    pub outcome: String,
    pub reason: String,
    pub created_at: u64,
}

const COLS: &str = "id, channel_h, expect_member, parent_hint, name, source, outcome, reason, \
                   created_at";

fn row_to_attempt(row: &rusqlite::Row) -> rusqlite::Result<ChannelReadinessAttempt> {
    Ok(ChannelReadinessAttempt {
        id: row.get(0)?,
        channel_h: row.get(1)?,
        expect_member: row.get(2)?,
        parent_hint: row.get(3)?,
        name: row.get(4)?,
        source: row.get(5)?,
        outcome: row.get(6)?,
        reason: row.get(7)?,
        created_at: row.get(8)?,
    })
}

impl Store {
    pub fn record_channel_readiness_attempt(
        &self,
        row: &NewChannelReadinessAttempt,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO channel_readiness_attempts
                 (channel_h, expect_member, parent_hint, name, source, outcome, reason, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.channel_h,
                row.expect_member,
                row.parent_hint,
                row.name,
                row.source,
                row.outcome,
                row.reason,
                row.created_at,
            ],
        )?;
        let inserted = self.conn.last_insert_rowid();
        let oldest = row.created_at.saturating_sub(RETAIN_FOR_SECS);
        self.conn.execute(
            "DELETE FROM channel_readiness_attempts
             WHERE created_at < ?1
                OR id NOT IN (
                    SELECT id FROM channel_readiness_attempts
                    ORDER BY created_at DESC, id DESC LIMIT ?2
                )",
            params![oldest, MAX_RETAINED_ATTEMPTS],
        )?;
        Ok(inserted)
    }

    pub fn channel_readiness_attempts(
        &self,
        channel_h: &str,
        limit: u32,
    ) -> Result<Vec<ChannelReadinessAttempt>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM channel_readiness_attempts
             WHERE channel_h=?1
             ORDER BY created_at DESC, id DESC
             LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![channel_h, limit], row_to_attempt)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn channel_readiness_attempt(&self, id: i64) -> Result<Option<ChannelReadinessAttempt>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM channel_readiness_attempts WHERE id=?1"),
                params![id],
                row_to_attempt,
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(created_at: u64) -> NewChannelReadinessAttempt {
        NewChannelReadinessAttempt {
            channel_h: "room".into(),
            expect_member: String::new(),
            parent_hint: None,
            name: None,
            source: "test".into(),
            outcome: "ready".into(),
            reason: "test".into(),
            created_at,
        }
    }

    #[test]
    fn readiness_history_keeps_only_one_hour_and_the_newest_five_hundred_rows() {
        let store = Store::open_memory().unwrap();
        store.record_channel_readiness_attempt(&attempt(1)).unwrap();
        for created_at in 10_000..10_510 {
            store
                .record_channel_readiness_attempt(&attempt(created_at))
                .unwrap();
        }

        let retained = store.channel_readiness_attempts("room", 1_000).unwrap();
        assert_eq!(retained.len(), MAX_RETAINED_ATTEMPTS as usize);
        assert_eq!(retained.first().unwrap().created_at, 10_509);
        assert_eq!(retained.last().unwrap().created_at, 10_010);
        assert!(store.channel_readiness_attempt(1).unwrap().is_none());
    }
}
