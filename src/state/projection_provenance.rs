//! Exact NMP ownership for rebuildable relay projections.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionKind {
    Channel,
    Profile,
    StatusSet,
    Event,
    Reaction,
}

impl ProjectionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Profile => "profile",
            Self::StatusSet => "status_set",
            Self::Event => "event",
            Self::Reaction => "reaction",
        }
    }
}

impl Store {
    pub(crate) fn begin_projection_frame(
        &self,
        observation_id: &str,
        generation: u64,
        evidence_json: &str,
        relay_settled: bool,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT INTO relay_projection_observations
                 (observation_id, generation, evidence_json, relay_settled)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(observation_id) DO UPDATE SET
                 generation=excluded.generation,
                 evidence_json=excluded.evidence_json,
                 relay_settled=excluded.relay_settled
             WHERE excluded.generation >= relay_projection_observations.generation",
            params![observation_id, generation, evidence_json, relay_settled],
        )?;
        Ok(changed > 0)
    }

    pub(crate) fn claim_projection_event(
        &self,
        observation_id: &str,
        generation: u64,
        event_id: &str,
        sources_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO relay_projection_owners
                 (observation_id, event_id, generation, sources_json)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(observation_id, event_id) DO UPDATE SET
                 generation=excluded.generation,
                 sources_json=excluded.sources_json",
            params![observation_id, event_id, generation, sources_json],
        )?;
        Ok(())
    }

    pub(crate) fn grow_projection_event_sources(
        &self,
        observation_id: &str,
        event_id: &str,
        sources_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE relay_projection_owners SET sources_json=?3
             WHERE observation_id=?1 AND event_id=?2",
            params![observation_id, event_id, sources_json],
        )?;
        Ok(())
    }

    pub(crate) fn release_projection_event(
        &self,
        observation_id: &str,
        event_id: &str,
    ) -> Result<bool> {
        self.conn.execute(
            "DELETE FROM relay_projection_owners
             WHERE observation_id=?1 AND event_id=?2",
            params![observation_id, event_id],
        )?;
        self.projection_event_is_orphaned(event_id)
    }

    pub(crate) fn settle_projection_frame(
        &self,
        observation_id: &str,
        generation: u64,
    ) -> Result<Vec<String>> {
        let stale = self.owner_event_ids(
            "SELECT event_id FROM relay_projection_owners
             WHERE observation_id=?1 AND generation<>?2",
            params![observation_id, generation],
        )?;
        self.conn.execute(
            "DELETE FROM relay_projection_owners
             WHERE observation_id=?1 AND generation<>?2",
            params![observation_id, generation],
        )?;
        self.orphaned(stale)
    }

    pub(crate) fn close_projection_observation(&self, observation_id: &str) -> Result<Vec<String>> {
        let owned = self.owner_event_ids(
            "SELECT event_id FROM relay_projection_owners WHERE observation_id=?1",
            [observation_id],
        )?;
        self.conn.execute(
            "DELETE FROM relay_projection_owners WHERE observation_id=?1",
            [observation_id],
        )?;
        self.conn.execute(
            "DELETE FROM relay_projection_observations WHERE observation_id=?1",
            [observation_id],
        )?;
        self.orphaned(owned)
    }

    pub(crate) fn set_projection_source(
        &self,
        kind: ProjectionKind,
        key: &str,
        source_event_id: &str,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM relay_projection_rows
             WHERE projection_kind=?1 AND projection_key=?2",
            params![kind.as_str(), key],
        )?;
        self.conn.execute(
            "INSERT INTO relay_projection_rows
                 (projection_kind, projection_key, source_event_id)
             VALUES (?1, ?2, ?3)",
            params![kind.as_str(), key, source_event_id],
        )?;
        Ok(())
    }

    pub(crate) fn retract_projection_source(&self, event_id: &str) -> Result<bool> {
        if !self.projection_event_is_orphaned(event_id)? {
            return Ok(false);
        }
        let rows = self.projection_rows_for_source(event_id)?;
        self.conn.execute(
            "DELETE FROM relay_projection_rows WHERE source_event_id=?1",
            [event_id],
        )?;
        for (kind, key) in &rows {
            if self.projection_has_sources(kind, key)? {
                continue;
            }
            match kind.as_str() {
                "channel" => {
                    self.conn
                        .execute("DELETE FROM relay_channels WHERE channel_h=?1", [key])?;
                }
                "profile" => {
                    self.conn
                        .execute("DELETE FROM relay_profiles WHERE pubkey=?1", [key])?;
                }
                "status_set" => {
                    self.conn
                        .execute("DELETE FROM relay_status WHERE pubkey=?1", [key])?;
                    self.conn
                        .execute("DELETE FROM relay_status_sets WHERE pubkey=?1", [key])?;
                }
                "event" => {
                    self.conn
                        .execute("DELETE FROM relay_events WHERE id=?1", [key])?;
                }
                "reaction" => {
                    self.conn
                        .execute("DELETE FROM relay_reactions WHERE reaction_id=?1", [key])?;
                }
                unknown => anyhow::bail!("unknown relay projection kind {unknown:?}"),
            }
        }
        Ok(!rows.is_empty())
    }

    fn projection_rows_for_source(&self, event_id: &str) -> Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT projection_kind, projection_key FROM relay_projection_rows
             WHERE source_event_id=?1",
        )?;
        Ok(statement
            .query_map([event_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn projection_has_sources(&self, kind: &str, key: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM relay_projection_rows
                     WHERE projection_kind=?1 AND projection_key=?2
                 )",
                params![kind, key],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    fn projection_event_is_orphaned(&self, event_id: &str) -> Result<bool> {
        Ok(!self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM relay_projection_owners WHERE event_id=?1
             )",
            [event_id],
            |row| row.get::<_, bool>(0),
        )?)
    }

    fn orphaned(&self, event_ids: Vec<String>) -> Result<Vec<String>> {
        event_ids
            .into_iter()
            .filter_map(
                |event_id| match self.projection_event_is_orphaned(&event_id) {
                    Ok(true) => Some(Ok(event_id)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                },
            )
            .collect()
    }

    fn owner_event_ids<P>(&self, sql: &str, params: P) -> Result<Vec<String>>
    where
        P: rusqlite::Params,
    {
        let mut statement = self.conn.prepare(sql)?;
        let event_ids = statement
            .query_map(params, |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(event_ids)
    }
}

#[cfg(test)]
mod tests;
