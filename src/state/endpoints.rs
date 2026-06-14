use super::*;

/// A registered TMUX (or future) endpoint for a session.
#[derive(Debug, Clone)]
pub struct SessionEndpoint {
    pub session_id: String,
    pub kind: String,   // "tmux"
    pub target: String, // stable pane id, e.g. "%5"
    pub meta: String,   // JSON: {"socket":"...", "pane_command":"claude"}
    pub registered_at: u64,
    pub last_verified: u64,
}

impl Store {
    // ── session_endpoints ─────────────────────────────────────────────────

    pub fn upsert_session_endpoint(
        &self,
        session_id: &str,
        kind: &str,
        target: &str,
        meta: &str,
        now: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_endpoints
               (session_id, kind, target, meta, registered_at, last_verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(session_id, kind) DO UPDATE SET
               target=?3, meta=?4, last_verified=?5",
            params![session_id, kind, target, meta, now],
        )?;
        Ok(())
    }

    pub fn get_session_endpoint(
        &self,
        session_id: &str,
        kind: &str,
    ) -> Result<Option<SessionEndpoint>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_id, kind, target, meta, registered_at, last_verified
                 FROM session_endpoints WHERE session_id=?1 AND kind=?2",
                params![session_id, kind],
                |r| {
                    Ok(SessionEndpoint {
                        session_id: r.get(0)?,
                        kind: r.get(1)?,
                        target: r.get(2)?,
                        meta: r.get(3)?,
                        registered_at: r.get(4)?,
                        last_verified: r.get(5)?,
                    })
                },
            )
            .ok())
    }

    /// All sessions that have an endpoint of the given kind.
    pub fn list_session_endpoints_of_kind(&self, kind: &str) -> Result<Vec<SessionEndpoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, kind, target, meta, registered_at, last_verified
             FROM session_endpoints WHERE kind=?1",
        )?;
        let rows = stmt
            .query_map(params![kind], |r| {
                Ok(SessionEndpoint {
                    session_id: r.get(0)?,
                    kind: r.get(1)?,
                    target: r.get(2)?,
                    meta: r.get(3)?,
                    registered_at: r.get(4)?,
                    last_verified: r.get(5)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn delete_session_endpoint(&self, session_id: &str, kind: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM session_endpoints WHERE session_id=?1 AND kind=?2",
            params![session_id, kind],
        )?;
        Ok(())
    }

    pub fn touch_session_endpoint_verified(
        &self,
        session_id: &str,
        kind: &str,
        now: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE session_endpoints SET last_verified=?3 WHERE session_id=?1 AND kind=?2",
            params![session_id, kind, now],
        )?;
        Ok(())
    }

    // ── project_paths ─────────────────────────────────────────────────────

    pub fn upsert_project_path(&self, project: &str, abs_path: &str, now: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO project_paths (project, abs_path, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(project) DO UPDATE SET abs_path=?2, updated_at=?3",
            params![project, abs_path, now],
        )?;
        Ok(())
    }

    pub fn get_project_path(&self, project: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT abs_path FROM project_paths WHERE project=?1",
                params![project],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }
}
