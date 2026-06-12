use super::*;

impl Store {
    // ── agent status ("what is X doing") ─────────────────────────────────

    pub fn set_agent_status(
        &self,
        pubkey: &str,
        project: &str,
        session_id: Option<&str>,
        text: &str,
        ts: u64,
    ) -> Result<()> {
        if let Some(session_id) = session_id.filter(|s| !s.is_empty()) {
            self.conn.execute(
                "INSERT INTO session_status (pubkey, project, session_id, text, updated_at)
                 VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(pubkey, project, session_id) DO UPDATE SET text=?4, updated_at=?5",
                params![pubkey, project, session_id, text, ts],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO agent_status (pubkey, project, text, updated_at) VALUES (?1,?2,?3,?4)
                 ON CONFLICT(pubkey, project) DO UPDATE SET text=?3, updated_at=?4",
                params![pubkey, project, text, ts],
            )?;
        }
        Ok(())
    }

    pub fn get_agent_status(
        &self,
        pubkey: &str,
        project: &str,
        session_id: Option<&str>,
    ) -> Result<Option<String>> {
        if let Some(session_id) = session_id.filter(|s| !s.is_empty()) {
            if let Some(text) = self
                .conn
                .query_row(
                    "SELECT text FROM session_status
                     WHERE pubkey=?1 AND project=?2 AND session_id=?3",
                    params![pubkey, project, session_id],
                    |r| r.get::<_, String>(0),
                )
                .ok()
            {
                return Ok(Some(text));
            }
        }
        Ok(self
            .conn
            .query_row(
                "SELECT text FROM agent_status WHERE pubkey=?1 AND project=?2",
                params![pubkey, project],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    // ── project metadata (NIP-29 kind 39000 cache) ───────────────────────

    pub fn upsert_project_meta(&self, project: &str, about: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO project_meta (project, about, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(project) DO UPDATE SET about=?2, updated_at=?3",
            params![project, about, ts],
        )?;
        Ok(())
    }

    pub fn get_project_meta(&self, project: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT about FROM project_meta WHERE project=?1",
                params![project],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    pub fn list_project_meta(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT project, about FROM project_meta ORDER BY project")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // ── NIP-29 owned groups + membership ─────────────────────────────────

    pub fn mark_group_owned(&self, project: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO owned_groups (project, created_at) VALUES (?1, ?2)
             ON CONFLICT(project) DO NOTHING",
            params![project, ts],
        )?;
        Ok(())
    }

    pub fn is_group_owned(&self, project: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM owned_groups WHERE project=?1",
            params![project],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn upsert_group_member(
        &self,
        project: &str,
        pubkey: &str,
        role: &str,
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO group_members (project, pubkey, role, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project, pubkey) DO UPDATE SET role=?3, updated_at=?4",
            params![project, pubkey, role, ts],
        )?;
        Ok(())
    }

    pub fn is_group_member(&self, project: &str, pubkey: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM group_members WHERE project=?1 AND pubkey=?2",
            params![project, pubkey],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Apply a relay-authoritative 39002 members snapshot for one group: replace
    /// the cached membership wholesale so we self-heal if our optimistic writes drifted.
    pub fn replace_group_members(
        &self,
        project: &str,
        members: &[(String, String)],
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM group_members WHERE project=?1",
            params![project],
        )?;
        for (pubkey, role) in members {
            self.conn.execute(
                "INSERT INTO group_members (project, pubkey, role, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(project, pubkey) DO UPDATE SET role=?3, updated_at=?4",
                params![project, pubkey, role, ts],
            )?;
        }
        Ok(())
    }

    pub fn is_mention_seen(&self, agent_pubkey: &str, event_id: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM seen_mentions WHERE agent_pubkey=?1 AND mention_event_id=?2",
            params![agent_pubkey, event_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}
