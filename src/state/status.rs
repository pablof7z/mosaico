//! `relay_status` — kind:30315 current activity keyed by `(pubkey, channel)`.

use super::*;

fn row_to_status(row: &rusqlite::Row) -> rusqlite::Result<Status> {
    Ok(Status {
        pubkey: row.get(0)?,
        channel_h: row.get(1)?,
        slug: row.get(2)?,
        title: row.get(3)?,
        activity: row.get(4)?,
        workspace: row.get(5)?,
        branch: row.get(6)?,
        state: crate::session_state::SessionState::parse(&row.get::<_, String>(7)?).ok_or_else(
            || rusqlite::Error::InvalidColumnType(7, "state".into(), rusqlite::types::Type::Text),
        )?,
        state_since: row.get(8)?,
        last_seen: row.get(9)?,
        updated_at: row.get(10)?,
        expiration: row.get(11)?,
    })
}

const COLS: &str = "pubkey, channel_h, slug, title, activity, workspace, branch, state, \
    state_since, last_seen, updated_at, expiration";
const UPSERT: &str = "INSERT INTO relay_status
        (pubkey, channel_h, slug, title, activity, workspace, branch,
         state, state_since, last_seen, updated_at, expiration)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
    ON CONFLICT(pubkey, channel_h) DO UPDATE SET
        slug=excluded.slug, title=excluded.title, activity=excluded.activity,
        workspace=excluded.workspace, branch=excluded.branch,
        state=excluded.state, state_since=excluded.state_since,
        last_seen=excluded.last_seen,
        updated_at=CASE
            WHEN relay_status.slug <> excluded.slug
              OR relay_status.title <> excluded.title
              OR relay_status.activity <> excluded.activity
              OR relay_status.workspace <> excluded.workspace
              OR relay_status.branch <> excluded.branch
              OR relay_status.state <> excluded.state
            THEN excluded.updated_at ELSE relay_status.updated_at END,
        expiration=excluded.expiration
    WHERE excluded.updated_at >= relay_status.updated_at";

impl Store {
    pub fn upsert_status(&self, status: &Status) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        upsert_status_row(&transaction, status)?;
        advance_status_set_watermark(&transaction, &status.pubkey, status.updated_at)?;
        transaction.commit()?;
        Ok(())
    }

    /// Atomically apply one replacement status event for a pubkey.
    ///
    /// The per-pubkey watermark makes a zero-channel replacement durable and
    /// prevents an older replacement from deleting or resurrecting rows.
    /// Returns `false` when the replacement lost the timestamp race.
    pub fn replace_status_channels(
        &self,
        pubkey: &str,
        statuses: &[Status],
        updated_at: u64,
    ) -> Result<bool> {
        anyhow::ensure!(!pubkey.trim().is_empty(), "status pubkey must not be empty");
        let mut channels = std::collections::BTreeSet::new();
        for status in statuses {
            anyhow::ensure!(
                status.pubkey == pubkey,
                "replacement status pubkey {:?} does not match {pubkey:?}",
                status.pubkey
            );
            anyhow::ensure!(
                status.updated_at == updated_at,
                "replacement status timestamp {} does not match {updated_at}",
                status.updated_at
            );
            anyhow::ensure!(
                channels.insert(status.channel_h.as_str()),
                "replacement status repeats channel {:?}",
                status.channel_h
            );
        }
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let watermark = transaction
            .query_row(
                "SELECT updated_at FROM relay_status_sets WHERE pubkey=?1",
                [pubkey],
                |row| row.get::<_, u64>(0),
            )
            .optional()?;
        if watermark.is_some_and(|current| current > updated_at) {
            transaction.rollback()?;
            return Ok(false);
        }
        let existing_channels = {
            let mut statement =
                transaction.prepare("SELECT channel_h FROM relay_status WHERE pubkey=?1")?;
            let rows = statement.query_map([pubkey], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for channel in existing_channels {
            if !channels.contains(channel.as_str()) {
                transaction.execute(
                    "DELETE FROM relay_status WHERE pubkey=?1 AND channel_h=?2",
                    params![pubkey, channel],
                )?;
            }
        }
        for status in statuses {
            upsert_status_row(&transaction, status)?;
        }
        advance_status_set_watermark(&transaction, pubkey, updated_at)?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn get_status(&self, pubkey: &str, channel_h: &str) -> Result<Option<Status>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM relay_status WHERE pubkey=?1 AND channel_h=?2"),
                params![pubkey, channel_h],
                row_to_status,
            )
            .optional()?)
    }

    pub fn live_status_for_channel(&self, channel_h: &str, now: u64) -> Result<Vec<Status>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_status
             WHERE channel_h=?1 AND expiration >= ?2 ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map(params![channel_h, now], row_to_status)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_status_sessions(
        &self,
        agent: Option<&str>,
        since: Option<u64>,
    ) -> Result<Vec<Status>> {
        let mut sql = format!("SELECT {COLS} FROM relay_status WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(agent) = agent.filter(|agent| !agent.is_empty()) {
            sql.push_str(" AND (pubkey=? OR slug=?)");
            args.push(Box::new(agent.to_string()));
            args.push(Box::new(agent.to_string()));
        }
        if let Some(since) = since {
            sql.push_str(" AND updated_at >= ?");
            args.push(Box::new(since as i64));
        }
        sql.push_str(" ORDER BY channel_h ASC, updated_at DESC");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), row_to_status)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn upsert_status_row(conn: &rusqlite::Connection, status: &Status) -> Result<()> {
    conn.execute(
        UPSERT,
        params![
            status.pubkey,
            status.channel_h,
            status.slug,
            status.title,
            status.activity,
            status.workspace,
            status.branch,
            status.state.as_str(),
            status.state_since,
            status.last_seen,
            status.updated_at,
            status.expiration,
        ],
    )?;
    Ok(())
}

fn advance_status_set_watermark(
    conn: &rusqlite::Connection,
    pubkey: &str,
    updated_at: u64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO relay_status_sets (pubkey, updated_at)
         VALUES (?1, ?2)
         ON CONFLICT(pubkey) DO UPDATE SET updated_at=excluded.updated_at
         WHERE excluded.updated_at >= relay_status_sets.updated_at",
        params![pubkey, updated_at],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "status/tests.rs"]
mod tests;
