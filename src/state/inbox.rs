use super::*;

impl Store {
    // ── inbox ────────────────────────────────────────────────────────────

    /// Idempotent insert. Returns true if the row was newly stored.
    pub fn enqueue_mention(&self, m: &InboxRow) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO inbox
               (mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, delivered, from_session)
             VALUES (?1,?2,?3,?4,?5,?6,?7,0,?8)",
            params![
                m.mention_event_id, m.target_session, m.from_pubkey, m.from_slug,
                m.project, m.body, m.created_at, m.from_session
            ],
        )?;
        Ok(changed > 0)
    }

    /// Pre-mark a (session_id, event_id) pair as already delivered so that any
    /// future `enqueue_mention` call for the same pair is a no-op and the event
    /// never surfaces in `drain_inbox`.  Used by `rpc_user_prompt` to prevent a
    /// relay echo of the published user-prompt event from appearing in the
    /// agent's own inbox.
    pub fn suppress_inbox_event(&self, session_id: &str, event_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO inbox
               (mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, delivered, from_session)
             VALUES (?1, ?2, '', '', '', '', 0, 1, '')",
            params![event_id, session_id],
        )?;
        Ok(())
    }

    /// Read undelivered mentions without marking them delivered. Safe for
    /// mid-turn checks (turn_check) — no writes to state.db.
    pub fn peek_inbox(&self, session_id: &str) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session
             FROM inbox WHERE target_session=?1 AND delivered=0 ORDER BY created_at",
        )?;
        let rows: Vec<InboxRow> = stmt
            .query_map(params![session_id], |row| {
                Ok(InboxRow {
                    mention_event_id: row.get(0)?,
                    target_session: row.get(1)?,
                    from_pubkey: row.get(2)?,
                    from_slug: row.get(3)?,
                    project: row.get(4)?,
                    body: row.get(5)?,
                    created_at: row.get(6)?,
                    from_session: row.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Peer sessions first seen at or after `since`, still live (last_seen >= fresh_since).
    pub fn list_new_peer_sessions(
        &self,
        since: u64,
        fresh_since: u64,
        project: Option<&str>,
    ) -> Result<Vec<PeerSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, pubkey, slug, project, host, last_seen, rel_cwd FROM peer_sessions
             WHERE first_seen>=?1 AND last_seen>=?2 AND (?3 IS NULL OR project=?3)
             ORDER BY first_seen",
        )?;
        let rows: Vec<PeerSession> = stmt
            .query_map(params![since, fresh_since, project], row_to_peer)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Agent status rows updated at or after `since`. Returns (slug, project, text).
    /// Resolves slug from profiles then peer_sessions, falling back to "unknown".
    pub fn list_status_changes_since(
        &self,
        since: u64,
        project: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(
                 (SELECT slug FROM profiles WHERE pubkey=ast.pubkey LIMIT 1),
                 (SELECT slug FROM peer_sessions WHERE pubkey=ast.pubkey ORDER BY last_seen DESC LIMIT 1),
                 'unknown'
             ), ast.project, ast.text, ast.updated_at
             FROM agent_status ast
             WHERE ast.updated_at>=?1 AND (?2 IS NULL OR ast.project=?2)
             UNION ALL
             SELECT COALESCE(
                 (SELECT agent_slug FROM sessions WHERE session_id=sst.session_id LIMIT 1),
                 (SELECT slug FROM peer_sessions WHERE session_id=sst.session_id LIMIT 1),
                 (SELECT slug FROM profiles WHERE pubkey=sst.pubkey LIMIT 1),
                 'unknown'
             ), sst.project, sst.text, sst.updated_at
             FROM session_status sst
             WHERE sst.updated_at>=?1 AND (?2 IS NULL OR sst.project=?2)
             ORDER BY 4",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![since, project], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // ── turn state (drives distillation) ─────────────────────────────────

    /// Mark a session as actively working on a turn, stamping its start time.
    /// Idempotent within a turn; a fresh `ts` signals a new turn to the engine.
    pub fn mark_turn_start(&self, session_id: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO turn_state (session_id, working, turn_started_at) VALUES (?1, 1, ?2)
             ON CONFLICT(session_id) DO UPDATE SET working=1, turn_started_at=?2",
            params![session_id, ts],
        )?;
        Ok(())
    }

    /// Mark a session idle (the turn ended). The engine publishes idle status on
    /// its next poll.
    pub fn mark_turn_end(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO turn_state (session_id, working, turn_started_at) VALUES (?1, 0, 0)
             ON CONFLICT(session_id) DO UPDATE SET working=0",
            params![session_id],
        )?;
        Ok(())
    }

    /// `(working, turn_started_at)` for a session. Defaults to `(false, 0)` when
    /// no turn has started yet, so the engine simply stays idle.
    pub fn get_turn_state(&self, session_id: &str) -> Result<(bool, u64)> {
        Ok(self
            .conn
            .query_row(
                "SELECT working, turn_started_at FROM turn_state WHERE session_id=?1",
                params![session_id],
                |r| Ok((r.get::<_, i64>(0)? != 0, r.get::<_, i64>(1)? as u64)),
            )
            .unwrap_or((false, 0)))
    }

    // ── per-agent mention dedup (across sessions) ────────────────────────

    pub fn mark_mention_seen(&self, agent_pubkey: &str, event_id: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO seen_mentions (agent_pubkey, mention_event_id, seen_at) VALUES (?1,?2,?3)",
            params![agent_pubkey, event_id, ts],
        )?;
        Ok(())
    }

    /// Return undelivered mentions for a session and mark them delivered.
    pub fn drain_inbox(&self, session_id: &str) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session
             FROM inbox WHERE target_session=?1 AND delivered=0 ORDER BY created_at",
        )?;
        let rows: Vec<InboxRow> = stmt
            .query_map(params![session_id], |row| {
                Ok(InboxRow {
                    mention_event_id: row.get(0)?,
                    target_session: row.get(1)?,
                    from_pubkey: row.get(2)?,
                    from_slug: row.get(3)?,
                    project: row.get(4)?,
                    body: row.get(5)?,
                    created_at: row.get(6)?,
                    from_session: row.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        self.conn.execute(
            "UPDATE inbox SET delivered=1 WHERE target_session=?1 AND delivered=0",
            params![session_id],
        )?;
        Ok(rows)
    }
}
