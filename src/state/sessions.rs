//! `sessions` — the local agent processes THIS daemon hosts.
//!
//! Canonical session identity is daemon-minted and stable. Harness-native ids are
//! aliases (see `aliases.rs`) that repoint to the newest live owner. EVERY
//! turn/session mutation resolves a raw external id to the canonical id BEFORE
//! writing — a known prior bug mutated by raw id and silently no-op'd.

use super::*;

const COLS: &str = "session_id, agent_pubkey, agent_slug, channel_h, harness, child_pid, \
     transcript_path, alive, created_at, last_seen, working, turn_started_at, last_distill_at, \
     seen_cursor, title, activity, resume_id, distill_fail_streak, distill_notice_at";

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
    Ok(Session {
        session_id: row.get(0)?,
        agent_pubkey: row.get(1)?,
        agent_slug: row.get(2)?,
        channel_h: row.get(3)?,
        harness: row.get(4)?,
        child_pid: row.get(5)?,
        transcript_path: row.get(6)?,
        alive: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
        last_seen: row.get(9)?,
        working: row.get::<_, i64>(10)? != 0,
        turn_started_at: row.get(11)?,
        last_distill_at: row.get(12)?,
        seen_cursor: row.get(13)?,
        title: row.get(14)?,
        activity: row.get(15)?,
        resume_id: row.get(16)?,
        distill_fail_streak: row.get(17)?,
        distill_notice_at: row.get(18)?,
    })
}

impl Store {
    /// Resolve any id (canonical session_id OR a harness-native external id) to
    /// the canonical session_id. Checks `sessions` first, then the newest alias
    /// pointing at it. The single chokepoint every session mutation funnels
    /// through so writes never miss a rotated id.
    pub(super) fn resolve_canonical_id(&self, id: &str) -> Result<Option<String>> {
        let direct: Option<String> = self
            .conn
            .query_row(
                "SELECT session_id FROM sessions WHERE session_id=?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        if direct.is_some() {
            return Ok(direct);
        }
        Ok(self
            .conn
            .query_row(
                "SELECT session_id FROM session_aliases WHERE external_id=?1
                 ORDER BY created_at DESC LIMIT 1",
                params![id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Resolve the canonical session id for an alias WITHOUT writing the session
    /// row — minting (and pointing the alias at) a fresh id when the alias is
    /// absent or its row was pruned. Splitting this out of [`Self::upsert_session_row`]
    /// lets `rpc_session_start` learn the id, select the ordinal signer (whose
    /// reservation is keyed by session id), and THEN write the row already
    /// carrying the correct ordinal pubkey rather than patching it afterward.
    pub fn resolve_or_mint_session_id(
        &self,
        harness: &str,
        external_id_kind: &str,
        external_id: &str,
        now: u64,
    ) -> Result<String> {
        let id = match self.resolve_session_by_alias(harness, external_id_kind, external_id)? {
            Some(id) if self.session_exists(&id)? => id,
            _ => mint_session_id(),
        };
        // (Re)point the external id at the resolved canonical session.
        self.put_alias(harness, external_id_kind, external_id, &id, now)?;
        Ok(id)
    }

    /// Insert or reassert the session row under an ALREADY-resolved canonical id
    /// (see [`Self::resolve_or_mint_session_id`]). `r.agent_pubkey` is written
    /// verbatim — the caller passes this session's selected ordinal pubkey, so a
    /// re-assert refreshes the row WITHOUT collapsing the ordinal back to the base
    /// (which would route a p-tagged mention to every ordinal of the agent).
    pub fn upsert_session_row(&self, session_id: &str, r: &RegisterSession) -> Result<()> {
        if self.session_exists(session_id)? {
            self.conn.execute(
                "UPDATE sessions SET agent_pubkey=?2, agent_slug=?3, channel_h=?4, harness=?5,
                     child_pid=?6, transcript_path=?7, resume_id=?8, alive=1, last_seen=?9
                 WHERE session_id=?1",
                params![
                    session_id,
                    r.agent_pubkey,
                    r.agent_slug,
                    r.channel_h,
                    r.harness,
                    r.child_pid,
                    r.transcript_path,
                    r.resume_id,
                    r.now
                ],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO sessions
                     (session_id, agent_pubkey, agent_slug, channel_h, harness, child_pid,
                      transcript_path, alive, created_at, last_seen, resume_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8, ?9)",
                params![
                    session_id,
                    r.agent_pubkey,
                    r.agent_slug,
                    r.channel_h,
                    r.harness,
                    r.child_pid,
                    r.transcript_path,
                    r.now,
                    r.resume_id
                ],
            )?;
        }
        if !r.channel_h.is_empty() {
            self.join_session_channel_canonical(session_id, &r.channel_h, r.now)?;
        }
        self.clear_session_claims_for_reassert(session_id, &r.agent_pubkey, &r.channel_h)?;
        Ok(())
    }

    /// Register or reassert a local session in one step: resolve/mint the id, then
    /// upsert the row with `r.agent_pubkey`. `rpc_session_start` uses the two-step
    /// form directly (it must select the signer between the two); other callers
    /// (tests, simple registrations) use this convenience wrapper.
    pub fn register_session(&self, r: &RegisterSession) -> Result<String> {
        let id = self.resolve_or_mint_session_id(
            &r.harness,
            &r.external_id_kind,
            &r.external_id,
            r.now,
        )?;
        self.upsert_session_row(&id, r)?;
        Ok(id)
    }

    fn session_exists(&self, session_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM sessions WHERE session_id=?1",
                params![session_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Fetch a session by any id (canonical or alias). Resolves the external id to
    /// the canonical session first.
    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(None);
        };
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM sessions WHERE session_id=?1"),
                params![canonical],
                row_to_session,
            )
            .optional()?)
    }

    /// All alive sessions on this machine, newest first.
    pub fn list_alive_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM sessions WHERE alive=1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt.query_map([], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Recent sessions (alive OR dead), newest first, capped. The resume picker's
    /// candidate set — a dead row with a resume token can be reconstituted.
    pub fn list_resumable_sessions(&self, limit: u32) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM sessions ORDER BY created_at DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![limit], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Find a session (alive or dead) whose canonical id starts with `prefix`,
    /// newest first. Used by resume flows to accept a short id prefix.
    pub fn find_session_by_prefix(&self, prefix: &str) -> Result<Option<Session>> {
        let pattern = format!("{}%", prefix.replace(['%', '_'], ""));
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM sessions WHERE session_id LIKE ?1
                     ORDER BY created_at DESC LIMIT 1"
                ),
                params![pattern],
                row_to_session,
            )
            .optional()?)
    }

    /// Set the working/turn flag for a session (resolves id first). When entering
    /// a turn, pass the start timestamp; when leaving, pass `working=false`.
    pub fn set_working(&self, id: &str, working: bool, turn_started_at: u64) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET working=?2, turn_started_at=?3 WHERE session_id=?1",
            params![canonical, working as i64, turn_started_at],
        )?;
        Ok(())
    }

    /// Bump a session's last_seen heartbeat (resolves first).
    pub fn touch_session(&self, id: &str, last_seen: u64) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET last_seen=?2 WHERE session_id=?1",
            params![canonical, last_seen],
        )?;
        Ok(())
    }

    /// Update a session's transcript path (resolves first). Set on turn start when
    /// the harness reports where its transcript lives.
    pub fn set_session_transcript(&self, id: &str, transcript_path: &str) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET transcript_path=?2 WHERE session_id=?1",
            params![canonical, transcript_path],
        )?;
        Ok(())
    }

    /// Realign a session row's wire identity to a re-selected ordinal pubkey.
    /// The normal start path is "born right" (the row is written with the ordinal
    /// pubkey via [`Self::upsert_session_row`]), so this is only for the reconcile
    /// path, which re-derives the signer for an already-registered session on
    /// daemon restart and keeps the row consistent with it. Resolves the id first.
    pub fn set_session_agent_pubkey(&self, id: &str, agent_pubkey: &str) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET agent_pubkey=?2 WHERE session_id=?1",
            params![canonical, agent_pubkey],
        )?;
        Ok(())
    }

    /// Move a session to a different channel/route scope (resolves first).
    pub fn set_session_channel(&self, id: &str, channel_h: &str) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET channel_h=?2 WHERE session_id=?1",
            params![canonical, channel_h],
        )?;
        if !channel_h.is_empty() {
            self.join_session_channel_canonical(&canonical, channel_h, crate::util::now_secs())?;
        }
        Ok(())
    }

    fn join_session_channel_canonical(
        &self,
        session_id: &str,
        channel_h: &str,
        joined_at: u64,
    ) -> Result<()> {
        if channel_h.trim().is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO session_channels (session_id, channel_h, joined_at)
             VALUES (?1, ?2, ?3)",
            params![session_id, channel_h, joined_at],
        )?;
        Ok(())
    }

    /// Join a channel for passive context and direct-mention delivery. The active
    /// publishing channel remains `sessions.channel_h`.
    pub fn join_session_channel(&self, id: &str, channel_h: &str, joined_at: u64) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.join_session_channel_canonical(&canonical, channel_h, joined_at)
    }

    /// Leave a passively joined channel. Callers must not use this to orphan the
    /// active publishing channel; `channels switch` handles active-channel moves.
    pub fn leave_session_channel(&self, id: &str, channel_h: &str) -> Result<bool> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(false);
        };
        let n = self.conn.execute(
            "DELETE FROM session_channels WHERE session_id=?1 AND channel_h=?2",
            params![canonical, channel_h],
        )?;
        Ok(n > 0)
    }

    /// True when the session listens to `channel_h`. The active route scope is
    /// always considered joined, even if the join row predates this schema.
    pub fn is_session_joined_channel(&self, id: &str, channel_h: &str) -> Result<bool> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(false);
        };
        if let Some(sess) = self.get_session(&canonical)? {
            if sess.channel_h == channel_h {
                return Ok(true);
            }
        }
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM session_channels WHERE session_id=?1 AND channel_h=?2",
                params![canonical, channel_h],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Joined channels for context/subscription coverage as `(channel_h,
    /// joined_at)`. The active channel is included as a compatibility fallback.
    pub fn list_session_joined_channels(&self, id: &str) -> Result<Vec<(String, u64)>> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT channel_h, joined_at FROM session_channels
             WHERE session_id=?1 ORDER BY joined_at ASC, channel_h ASC",
        )?;
        let rows = stmt.query_map(params![canonical.clone()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        let mut joined = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if let Some(sess) = self.get_session(&canonical)? {
            if !sess.channel_h.is_empty() && !joined.iter().any(|(h, _)| h == &sess.channel_h) {
                joined.push((sess.channel_h, sess.created_at));
            }
        }
        joined.sort_by(|(a_h, a_t), (b_h, b_t)| a_t.cmp(b_t).then(a_h.cmp(b_h)));
        Ok(joined)
    }

    /// Mark a session dead (process exited). Resolves the id first; clears the
    /// working flag.
    pub fn mark_dead(&self, id: &str) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE sessions SET alive=0, working=0 WHERE session_id=?1",
            params![canonical],
        )?;
        Ok(())
    }
}
