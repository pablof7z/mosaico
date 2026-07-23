//! `relay_status` — kind:30315 current activity keyed by `(pubkey, channel)`.

use super::*;

fn row_to_status(row: &rusqlite::Row) -> rusqlite::Result<Status> {
    Ok(Status {
        pubkey: row.get(0)?,
        channel_h: row.get(1)?,
        slug: row.get(2)?,
        title: row.get(3)?,
        activity: row.get(4)?,
        state: crate::session_state::SessionState::parse(&row.get::<_, String>(5)?).ok_or_else(
            || rusqlite::Error::InvalidColumnType(5, "state".into(), rusqlite::types::Type::Text),
        )?,
        state_since: row.get(6)?,
        last_seen: row.get(7)?,
        updated_at: row.get(8)?,
        expiration: row.get(9)?,
    })
}

const COLS: &str =
    "pubkey, channel_h, slug, title, activity, state, state_since, last_seen, updated_at, expiration";

impl Store {
    pub fn upsert_status(&self, status: &Status) -> Result<()> {
        self.conn.execute(
            "INSERT INTO relay_status
                 (pubkey, channel_h, slug, title, activity, state, state_since,
                  last_seen, updated_at, expiration)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(pubkey, channel_h) DO UPDATE SET
                 slug=excluded.slug, title=excluded.title, activity=excluded.activity,
                 state=excluded.state, state_since=excluded.state_since,
                 last_seen=excluded.last_seen,
                 updated_at=CASE
                     WHEN relay_status.slug <> excluded.slug
                       OR relay_status.title <> excluded.title
                       OR relay_status.activity <> excluded.activity
                       OR relay_status.state <> excluded.state
                     THEN excluded.updated_at ELSE relay_status.updated_at END,
                 expiration=excluded.expiration
             WHERE excluded.updated_at >= relay_status.updated_at",
            params![
                status.pubkey,
                status.channel_h,
                status.slug,
                status.title,
                status.activity,
                status.state.as_str(),
                status.state_since,
                status.last_seen,
                status.updated_at,
                status.expiration,
            ],
        )?;
        Ok(())
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

    pub fn active_channels_since(&self, cursor: u64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT channel_h FROM relay_status WHERE updated_at >= ?1 ORDER BY channel_h",
        )?;
        let rows = stmt.query_map([cursor], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::state::{Status, Store};

    fn status(
        activity: &str,
        state: crate::session_state::SessionState,
        updated_at: u64,
    ) -> Status {
        Status {
            pubkey: "pk".into(),
            channel_h: "h1".into(),
            slug: "agent".into(),
            title: "Task title".into(),
            activity: activity.into(),
            state,
            state_since: 100,
            last_seen: updated_at,
            updated_at,
            expiration: updated_at + 100,
        }
    }

    #[test]
    fn lease_renewal_refreshes_liveness_without_advancing_delta_clock() {
        let store = Store::open_memory().unwrap();
        store
            .upsert_status(&status(
                "reading",
                crate::session_state::SessionState::Working,
                100,
            ))
            .unwrap();
        store
            .upsert_status(&status(
                "reading",
                crate::session_state::SessionState::Working,
                150,
            ))
            .unwrap();
        let row = store.get_status("pk", "h1").unwrap().unwrap();
        assert_eq!(
            (
                row.state_since,
                row.last_seen,
                row.expiration,
                row.updated_at
            ),
            (100, 150, 250, 100)
        );
    }

    #[test]
    fn semantic_status_change_advances_delta_clock() {
        let store = Store::open_memory().unwrap();
        store
            .upsert_status(&status(
                "reading",
                crate::session_state::SessionState::Working,
                100,
            ))
            .unwrap();
        store
            .upsert_status(&status(
                "writing",
                crate::session_state::SessionState::Working,
                150,
            ))
            .unwrap();
        let row = store.get_status("pk", "h1").unwrap().unwrap();
        assert_eq!((row.activity.as_str(), row.updated_at), ("writing", 150));
        assert_eq!(row.state_since, 100);
    }
}
