use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandingState {
    Member,
    Absent,
}

impl StandingState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Absent => "absent",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "member" => Ok(Self::Member),
            "absent" => Ok(Self::Absent),
            _ => anyhow::bail!("unknown StandingState value {value:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStanding {
    pub pubkey: String,
    pub channel_h: String,
    pub state: StandingState,
    pub standing_epoch: u64,
    pub session_lifecycle_epoch: u64,
    pub updated_at: u64,
}

const COLS: &str = "pubkey, channel_h, state, standing_epoch, session_lifecycle_epoch, updated_at";

fn row_to_standing(row: &rusqlite::Row) -> rusqlite::Result<SessionStanding> {
    let raw: String = row.get(2)?;
    let state = StandingState::parse(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error.to_string())),
        )
    })?;
    Ok(SessionStanding {
        pubkey: row.get(0)?,
        channel_h: row.get(1)?,
        state,
        standing_epoch: row.get(3)?,
        session_lifecycle_epoch: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

impl Store {
    pub fn get_session_standing(
        &self,
        pubkey: &str,
        channel_h: &str,
    ) -> Result<Option<SessionStanding>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM session_standing WHERE pubkey=?1 AND channel_h=?2"),
                params![pubkey, channel_h],
                row_to_standing,
            )
            .optional()?)
    }

    pub fn list_session_standing(&self, pubkey: &str) -> Result<Vec<SessionStanding>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLS} FROM session_standing WHERE pubkey=?1 ORDER BY channel_h"
        ))?;
        let rows = statement.query_map([pubkey], row_to_standing)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Apply confirmed relay admission only while the same lifecycle still
    /// owns the request. Runtime stop does not revoke channel membership.
    pub fn mark_session_standing_member_if_running(
        &self,
        pubkey: &str,
        channel_h: &str,
        expected_lifecycle_epoch: u64,
        now: u64,
    ) -> Result<Option<u64>> {
        let changed = self.conn.execute(
            "INSERT INTO session_standing
                 (pubkey, channel_h, state, standing_epoch,
                  session_lifecycle_epoch, updated_at)
             SELECT ?1, ?2, 'member', 1, lifecycle_epoch, ?4
             FROM sessions
             WHERE pubkey=?1 AND runtime_state='running' AND lifecycle_epoch=?3
               AND recovery_state<>'revoked'
             ON CONFLICT(pubkey, channel_h) DO UPDATE SET
                 state='member',
                 standing_epoch=session_standing.standing_epoch + 1,
                 session_lifecycle_epoch=excluded.session_lifecycle_epoch,
                 updated_at=excluded.updated_at
             WHERE EXISTS (
                 SELECT 1 FROM sessions
                 WHERE pubkey=?1 AND runtime_state='running' AND lifecycle_epoch=?3
                   AND recovery_state<>'revoked'
             )",
            params![pubkey, channel_h, expected_lifecycle_epoch, now],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(Some(self.conn.query_row(
            "SELECT standing_epoch FROM session_standing WHERE pubkey=?1 AND channel_h=?2",
            params![pubkey, channel_h],
            |row| row.get(0),
        )?))
    }

    /// Relay membership that has no matching session route is compensation
    /// work from a failed or stale admission commit.
    pub fn list_cleanup_due_member_standing(&self) -> Result<Vec<SessionStanding>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLS}
               FROM session_standing standing
              WHERE standing.state='member'
                AND NOT EXISTS (
                    SELECT 1 FROM session_channels route
                     WHERE route.pubkey=standing.pubkey
                       AND route.channel_h=standing.channel_h
                )
              ORDER BY standing.updated_at, standing.pubkey, standing.channel_h"
        ))?;
        let rows = statement.query_map([], row_to_standing)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_stopped_member_standing(&self) -> Result<Vec<SessionStanding>> {
        let mut statement = self.conn.prepare(
            "SELECT standing.pubkey, standing.channel_h, standing.state,
                    standing.standing_epoch, standing.session_lifecycle_epoch,
                    standing.updated_at
               FROM session_standing standing
               JOIN sessions session ON session.pubkey=standing.pubkey
               JOIN session_channels route
                 ON route.pubkey=standing.pubkey
                AND route.channel_h=standing.channel_h
              WHERE standing.state='member' AND session.runtime_state='stopped'
              ORDER BY standing.updated_at DESC, standing.pubkey, standing.channel_h",
        )?;
        let rows = statement.query_map([], row_to_standing)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn mark_member_standing_absent_if_epoch(
        &self,
        pubkey: &str,
        channel_h: &str,
        standing_epoch: u64,
        session_lifecycle_epoch: u64,
        now: u64,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE session_standing
             SET state='absent', standing_epoch=standing_epoch+1, updated_at=?5
             WHERE pubkey=?1 AND channel_h=?2 AND state='member'
               AND standing_epoch=?3 AND session_lifecycle_epoch=?4",
            params![
                pubkey,
                channel_h,
                standing_epoch,
                session_lifecycle_epoch,
                now
            ],
        )? == 1)
    }
}

#[cfg(test)]
#[path = "session_standing/tests.rs"]
mod tests;
