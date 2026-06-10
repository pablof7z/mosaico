use super::*;

impl Store {
    // ── sessions (mine) ──────────────────────────────────────────────────

    pub fn upsert_session(&self, r: &SessionRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions
               (session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(session_id) DO UPDATE SET
               agent_slug=?2, agent_pubkey=?3, project=?4, host=?5,
               child_pid=?6, watch_pid=?7, alive=?9, rel_cwd=?10",
            params![
                r.session_id, r.agent_slug, r.agent_pubkey, r.project, r.host,
                r.child_pid, r.watch_pid, r.created_at, r.alive as i32, r.rel_cwd
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> Result<Option<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd
             FROM sessions WHERE session_id=?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_session(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_alive_sessions(&self) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd
             FROM sessions WHERE alive=1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |row| row_to_session(row))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Most-recent still-alive session for a project — lets an agent that
    /// doesn't know its session id resolve "my session" from the cwd.
    pub fn latest_alive_session_for_project(&self, project: &str) -> Result<Option<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd
             FROM sessions WHERE alive=1 AND project=?1 ORDER BY created_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![project])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_session(row)?))
        } else {
            Ok(None)
        }
    }

    /// Most-recent still-alive session for a SPECIFIC agent in a project. Used by
    /// session resolution so a sender's identity is scoped to the invoking agent
    /// (`$TENEX_EDGE_AGENT`) rather than falling back to whatever agent was most
    /// recently active in the project — which would sign/record a `claude` send as
    /// `opencode` if opencode happened to be the latest-active session.
    pub fn latest_alive_session_for_agent_in_project(
        &self,
        agent_slug: &str,
        project: &str,
    ) -> Result<Option<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd
             FROM sessions WHERE alive=1 AND project=?1 AND agent_slug=?2 ORDER BY created_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![project, agent_slug])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_session(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn mark_session_dead(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET alive=0 WHERE session_id=?1",
            params![id],
        )?;
        Ok(())
    }

    /// Record the host transcript path for a session (provided by the hook), so
    /// the engine can read the recent conversation to distill activity.
    pub fn set_session_transcript(&self, id: &str, path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET transcript_path=?2 WHERE session_id=?1",
            params![id, path],
        )?;
        Ok(())
    }

    pub fn get_session_transcript(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT transcript_path FROM sessions WHERE session_id=?1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten())
    }

    /// Returns `(thread_root_event_id, last_prompt_event_id)` for a session.
    /// Both are empty strings until the first user prompt is published.
    pub fn get_thread_event_ids(&self, session_id: &str) -> (String, String) {
        self.conn
            .query_row(
                "SELECT thread_root_event_id, last_prompt_event_id FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .unwrap_or_default()
    }

    /// Update the NIP-10 thread tracking for a session.
    /// `root_id` is the first user prompt event; `prompt_id` is the most recent.
    pub fn set_thread_event_ids(&self, session_id: &str, root_id: &str, prompt_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET thread_root_event_id=?2, last_prompt_event_id=?3 WHERE session_id=?1",
            params![session_id, root_id, prompt_id],
        )?;
        Ok(())
    }

    /// Snapshot the last assistant text at the start of a turn. `rpc_turn_end`
    /// polls until the transcript returns something *different* from this value,
    /// so it reliably reads the current turn's response even when Claude Code
    /// writes the transcript after the stop hook fires.
    pub fn set_last_assistant_text_at_turn_start(&self, session_id: &str, text: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_assistant_text_at_turn_start=?2 WHERE session_id=?1",
            params![session_id, text],
        )?;
        Ok(())
    }

    pub fn get_last_assistant_text_at_turn_start(&self, session_id: &str) -> String {
        self.conn
            .query_row(
                "SELECT last_assistant_text_at_turn_start FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_default()
    }

    /// Heartbeat: keep a live session's `last_seen` fresh (called each engine tick).
    pub fn touch_session(&self, id: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_seen=?2 WHERE session_id=?1",
            params![id, ts],
        )?;
        Ok(())
    }

    pub fn session_last_seen(&self, id: &str) -> Result<Option<u64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT last_seen FROM sessions WHERE session_id=?1",
                params![id],
                |r| r.get::<_, u64>(0),
            )
            .ok())
    }

    /// My own sessions whose heartbeat is fresh (alive + recently touched).
    pub fn list_my_live_sessions(&self, since: u64) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd
             FROM sessions WHERE alive=1 AND last_seen>=?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![since], |row| row_to_session(row))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
