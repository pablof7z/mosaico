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

    /// Prune this backend's own cached rows down to `keep_slugs`, dropping any
    /// `(backend_pubkey, agent_slug)` whose slug is no longer advertised. Lets a
    /// deleted agent leave the local cache immediately instead of waiting on the
    /// best-effort async 30555 tombstone round-trip (which can fail to publish,
    /// miss the advertised address, or be lost across a restart). Strictly
    /// scoped to `backend_pubkey`, so other backends' advertisements are never
    /// touched. Returns the number of slugs removed.
    pub fn retain_local_agent_roster(
        &self,
        backend_pubkey: &str,
        keep_slugs: &[String],
    ) -> Result<usize> {
        let keep: std::collections::BTreeSet<&str> =
            keep_slugs.iter().map(String::as_str).collect();
        let existing = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT agent_slug FROM relay_agent_roster WHERE backend_pubkey=?1",
            )?;
            let rows = stmt
                .query_map(params![backend_pubkey], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let stale = existing
            .into_iter()
            .filter(|slug| !keep.contains(slug.as_str()))
            .collect::<Vec<_>>();
        for slug in &stale {
            self.conn.execute(
                "DELETE FROM relay_agent_roster WHERE backend_pubkey=?1 AND agent_slug=?2",
                params![backend_pubkey, slug],
            )?;
        }
        Ok(stale.len())
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

    #[test]
    fn retain_local_prunes_unadvertised_slugs_for_own_backend_only() {
        let s = Store::open_memory().unwrap();
        for (backend, slug) in [("mine", "kept"), ("mine", "deleted"), ("other", "deleted")] {
            s.replace_agent_roster(&AgentRoster {
                backend_pubkey: backend.into(),
                host: "laptop".into(),
                slug: slug.into(),
                use_criteria: "x".into(),
                channels: vec!["root".into()],
                updated_at: 1,
            })
            .unwrap();
        }

        let removed = s
            .retain_local_agent_roster("mine", &["kept".to_string()])
            .unwrap();
        assert_eq!(removed, 1);

        let slugs = s
            .list_agent_roster()
            .unwrap()
            .into_iter()
            .map(|r| (r.backend_pubkey, r.slug))
            .collect::<std::collections::BTreeSet<_>>();
        // Own unadvertised slug dropped; own kept slug and the *other* backend's
        // identically-named row both survive.
        assert!(!slugs.contains(&("mine".into(), "deleted".into())));
        assert!(slugs.contains(&("mine".into(), "kept".into())));
        assert!(slugs.contains(&("other".into(), "deleted".into())));
    }
}
