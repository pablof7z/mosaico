//! `relay_agent_roster` — kind:30555 backend capability advertisements.
//!
//! The backend management key signs one addressable event per capability slug.
//! Repeated `h` tags fan out to one cached row per root channel.

use super::*;

/// kind:30555 backend-published capability roster, fanned out to one row per
/// advertised root channel. The signer is the backend management key; `slug` is
/// the capability label, not an agent identity pubkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAvailability {
    pub backend_pubkey: String,
    pub host: String,
    pub slug: String,
    pub use_criteria: String,
    pub channel_h: String,
    pub updated_at: u64,
}

/// Complete replacement payload for one `(backend_pubkey, slug)` 30555 address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRoster {
    pub backend_pubkey: String,
    pub host: String,
    pub slug: String,
    pub use_criteria: String,
    pub channels: Vec<String>,
    pub updated_at: u64,
}

const COLS: &str = "backend_pubkey, host, agent_slug, use_criteria, channel_h, updated_at";

fn row_to_availability(row: &rusqlite::Row) -> rusqlite::Result<AgentAvailability> {
    Ok(AgentAvailability {
        backend_pubkey: row.get(0)?,
        host: row.get(1)?,
        slug: row.get(2)?,
        use_criteria: row.get(3)?,
        channel_h: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

impl Store {
    /// Replace the materialized rows for one `(backend_pubkey, slug)` roster
    /// address. Older replacement events are ignored wholesale.
    pub fn replace_agent_roster(&self, roster: &AgentRoster) -> Result<()> {
        let newest: Option<u64> = self
            .conn
            .query_row(
                "SELECT MAX(updated_at) FROM relay_agent_roster
                 WHERE backend_pubkey=?1 AND agent_slug=?2",
                params![roster.backend_pubkey, roster.slug],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if newest.is_some_and(|ts| ts > roster.updated_at) {
            return Ok(());
        }

        self.conn.execute(
            "DELETE FROM relay_agent_roster
             WHERE backend_pubkey=?1 AND agent_slug=?2",
            params![roster.backend_pubkey, roster.slug],
        )?;

        let mut channels = roster
            .channels
            .iter()
            .map(|h| h.trim())
            .filter(|h| !h.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        channels.sort();
        channels.dedup();

        for channel in channels {
            self.conn.execute(
                "INSERT INTO relay_agent_roster
                     (backend_pubkey, agent_slug, channel_h, host, use_criteria, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    roster.backend_pubkey,
                    roster.slug,
                    channel,
                    roster.host,
                    roster.use_criteria,
                    roster.updated_at
                ],
            )?;
        }
        Ok(())
    }

    /// Agent capabilities advertised for a root channel by every backend whose
    /// 30555 event has materialized locally.
    pub fn list_agent_roster_for_channel(&self, channel_h: &str) -> Result<Vec<AgentAvailability>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_agent_roster
             WHERE channel_h=?1
             ORDER BY host ASC, agent_slug ASC, backend_pubkey ASC"
        ))?;
        let rows = stmt.query_map(params![channel_h], row_to_availability)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Full materialized capability list, grouped by backend/slug/channel.
    pub fn list_agent_roster(&self) -> Result<Vec<AgentAvailability>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_agent_roster
             ORDER BY channel_h ASC, host ASC, agent_slug ASC, backend_pubkey ASC"
        ))?;
        let rows = stmt.query_map([], row_to_availability)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_replacement_fans_out_and_removes_old_channels() {
        let s = Store::open_memory().unwrap();
        s.replace_agent_roster(&AgentRoster {
            backend_pubkey: "backend".into(),
            host: "laptop".into(),
            slug: "codex".into(),
            use_criteria: "For coding".into(),
            channels: vec!["root-a".into(), "root-b".into()],
            updated_at: 10,
        })
        .unwrap();
        assert_eq!(s.list_agent_roster_for_channel("root-a").unwrap().len(), 1);
        assert_eq!(s.list_agent_roster_for_channel("root-b").unwrap().len(), 1);

        s.replace_agent_roster(&AgentRoster {
            backend_pubkey: "backend".into(),
            host: "laptop".into(),
            slug: "codex".into(),
            use_criteria: "For coding".into(),
            channels: vec!["root-b".into()],
            updated_at: 11,
        })
        .unwrap();
        assert!(s
            .list_agent_roster_for_channel("root-a")
            .unwrap()
            .is_empty());
        assert_eq!(s.list_agent_roster_for_channel("root-b").unwrap().len(), 1);
    }
}
