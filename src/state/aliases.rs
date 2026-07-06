//! `session_aliases` — external id -> canonical session (N:1, repointable).
//!
//! Reused OS slots (PTY endpoint, watch pid) and rotated harness ids repoint to the
//! newest live owner. Keyed by `(harness, external_id_kind, external_id)`.

use super::*;

impl Store {
    /// Point an external id at a canonical session. Re-pointing a reused slot or a
    /// rotated harness id is an ON CONFLICT update to the newest owner.
    pub fn put_alias(
        &self,
        harness: &str,
        external_id_kind: &str,
        external_id: &str,
        session_id: &str,
        created_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_aliases
                 (harness, external_id_kind, external_id, session_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(harness, external_id_kind, external_id)
                 DO UPDATE SET session_id=excluded.session_id, created_at=excluded.created_at",
            params![
                harness,
                external_id_kind,
                external_id,
                session_id,
                created_at
            ],
        )?;
        Ok(())
    }

    /// Resolve an external id of a SPECIFIC kind to its newest ALIVE session.
    /// Type-safe (matches `external_id_kind`, not just the raw id) and never
    /// returns a dead row — the in-session anchors (`pty_session`,
    /// `harness_session`) must resolve to a LIVE session, never a ghost whose
    /// alias has not yet been repointed.
    ///
    /// `harness` full-keys the match `(harness, kind, external_id)` per the alias
    /// schema. Pass `Some` for harness-native ids (a harness session id is only
    /// unique within its harness); pass `None` for `pty_session`, whose ids are
    /// host-local endpoint ids.
    pub fn alive_session_for_alias(
        &self,
        harness: Option<&str>,
        external_id_kind: &str,
        external_id: &str,
    ) -> Result<Option<Session>> {
        let id: Option<String> = match harness {
            Some(h) => self
                .conn
                .query_row(
                    "SELECT a.session_id FROM session_aliases a
                     JOIN sessions s ON s.session_id = a.session_id
                     WHERE a.harness=?1 AND a.external_id_kind=?2 AND a.external_id=?3
                       AND s.alive=1
                     ORDER BY a.created_at DESC LIMIT 1",
                    params![h, external_id_kind, external_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?,
            None => self
                .conn
                .query_row(
                    "SELECT a.session_id FROM session_aliases a
                     JOIN sessions s ON s.session_id = a.session_id
                     WHERE a.external_id_kind=?1 AND a.external_id=?2 AND s.alive=1
                     ORDER BY a.created_at DESC LIMIT 1",
                    params![external_id_kind, external_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?,
        };
        match id {
            Some(id) => self.get_session(&id),
            None => Ok(None),
        }
    }

    /// Resolve a specific external id (by its full key) to its canonical session.
    pub fn resolve_session_by_alias(
        &self,
        harness: &str,
        external_id_kind: &str,
        external_id: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_id FROM session_aliases
                 WHERE harness=?1 AND external_id_kind=?2 AND external_id=?3",
                params![harness, external_id_kind, external_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// All aliases of a given kind across every session, newest first (e.g. all
    /// `pty_session` bindings to enumerate live PTY endpoints).
    pub fn list_aliases_of_kind(&self, external_id_kind: &str) -> Result<Vec<SessionAlias>> {
        let mut stmt = self.conn.prepare(
            "SELECT harness, external_id_kind, external_id, session_id, created_at
             FROM session_aliases WHERE external_id_kind=?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![external_id_kind], |row| {
            Ok(SessionAlias {
                harness: row.get(0)?,
                external_id_kind: row.get(1)?,
                external_id: row.get(2)?,
                session_id: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Drop the `pty_session` alias(es) for a session (resolves the id first). Used
    /// when the bound PTY is found dead, so it is no longer treated as an endpoint.
    pub fn clear_pty_session(&self, session_id: &str) -> Result<()> {
        self.clear_alias_kind(session_id, "pty_session")
    }

    /// Drop a specific alias kind for a session after its endpoint is found dead.
    pub fn clear_alias_kind(&self, session_id: &str, external_id_kind: &str) -> Result<()> {
        let target = self
            .resolve_canonical_id(session_id)?
            .unwrap_or_else(|| session_id.to_string());
        self.conn.execute(
            "DELETE FROM session_aliases WHERE session_id=?1 AND external_id_kind=?2",
            params![target, external_id_kind],
        )?;
        Ok(())
    }

    /// All external-id aliases pointing at a canonical session (e.g. to retire
    /// them when the session dies).
    pub fn aliases_for_session(&self, session_id: &str) -> Result<Vec<SessionAlias>> {
        let mut stmt = self.conn.prepare(
            "SELECT harness, external_id_kind, external_id, session_id, created_at
             FROM session_aliases WHERE session_id=?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(SessionAlias {
                harness: row.get(0)?,
                external_id_kind: row.get(1)?,
                external_id: row.get(2)?,
                session_id: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
