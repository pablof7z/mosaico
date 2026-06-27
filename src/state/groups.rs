use super::*;

impl Store {
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

    /// Human-readable display name of a group/channel: the NIP-29 `name` tag
    /// from kind:39000 if known, else the `about` text, else empty. Source of
    /// truth for the statusline channel title (== the channel's title on the
    /// relay == what the relay renders as the room's name). Pure read.
    pub fn group_display_name(&self, project: &str) -> Result<String> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT name, about FROM project_meta WHERE project=?1",
                params![project],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .ok();
        Ok(row
            .map(|(name, about)| if !name.is_empty() { name } else { about })
            .unwrap_or_default())
    }

    /// Latest non-empty title for a local session whose routing room is `project`.
    /// Used after an asynchronous per-session room create succeeds: the title may
    /// have been seeded while the relay was still minting the group.
    pub fn latest_session_title_for_project(&self, project: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT title FROM session_state
                 WHERE project=?1 AND title <> ''
                 ORDER BY updated_at DESC LIMIT 1",
                params![project],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .context("querying latest session title for project")
    }

    /// Record a group's NIP-29 subgroup hierarchy (display `name` + `parent` id)
    /// from its relay-authored kind:39000, without disturbing its `about`. Keyed
    /// by group id; coexists with `upsert_project_meta` on the same row.
    pub fn upsert_group_metadata(
        &self,
        project: &str,
        name: &str,
        parent: &str,
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO project_meta (project, about, name, parent, updated_at)
             VALUES (?1, '', ?2, ?3, ?4)
             ON CONFLICT(project) DO UPDATE SET
               name=?2,
               parent=CASE WHEN ?3='' THEN parent ELSE ?3 END,
               updated_at=?4",
            params![project, name, parent, ts],
        )?;
        Ok(())
    }

    /// All known group metadata rows `(group_id, about, name, parent)`. Source of
    /// truth for `groups list`'s hierarchy — purely local, no relay round-trip.
    pub fn list_group_metadata(&self) -> Result<Vec<(String, String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT project, about, name, parent FROM project_meta")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// The parent group id of `group`, or `None` when it is a top-level project
    /// group (empty parent) or unknown. A non-empty parent means `group` is a
    /// subgroup (a per-session room or a task room).
    pub fn group_parent(&self, group: &str) -> Result<Option<String>> {
        let parent: rusqlite::Result<String> = self.conn.query_row(
            "SELECT parent FROM project_meta WHERE project=?1",
            params![group],
            |r| r.get::<_, String>(0),
        );
        match parent {
            Ok(p) if !p.is_empty() => Ok(Some(p)),
            Ok(_) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
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
            "INSERT INTO owned_groups (project, created_at, owns_group) VALUES (?1, ?2, 1)
             ON CONFLICT(project) DO UPDATE SET owns_group=1",
            params![project, ts],
        )?;
        Ok(())
    }

    /// Every group id this daemon owns (`owns_group=1`). Feeds the aggregate
    /// subscription registry's coverage so the daemon keeps live `#h`/group-state
    /// REQs for groups it created even when no local agent is a cached member yet.
    pub fn list_owned_groups(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT project FROM owned_groups WHERE owns_group=1")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn is_group_owned(&self, project: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM owned_groups WHERE project=?1 AND owns_group=1",
            params![project],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// True if this add-agents orchestration event id was already processed
    /// (durable dedup; see `processed_orchestration`). Errors are swallowed to
    /// `false` so a transient DB hiccup re-processes rather than silently drops.
    /// Atomically CLAIM an orchestration event for processing. Returns `true`
    /// iff THIS call inserted the row — i.e. no concurrent/earlier delivery had
    /// already claimed it. The relay fans the same kind:9 out across every
    /// matching subscription, so the handler body must run AT MOST ONCE; the
    /// single-writer store + `INSERT OR IGNORE` serialize that race. Survives
    /// restarts, so a replayed event never re-provisions.
    pub fn try_claim_orchestration(&self, event_id: &str, ts: u64) -> bool {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO processed_orchestration (event_id, processed_at)
                 VALUES (?1, ?2)",
                params![event_id, ts],
            )
            .map(|n| n == 1)
            .unwrap_or(false)
    }

    /// Release a claim so a later redelivery can retry — used when provisioning
    /// fails in a way that may succeed next time (e.g. a transient relay reject).
    pub fn unclaim_orchestration(&self, event_id: &str) {
        let _ = self.conn.execute(
            "DELETE FROM processed_orchestration WHERE event_id=?1",
            params![event_id],
        );
    }

    /// Cached NIP-29 roster size for a project (0 when membership is unknown,
    /// e.g. no tenexPrivateKey → no group management → empty cache).
    pub fn count_group_members(&self, project: &str) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM group_members WHERE project=?1",
            params![project],
            |r| r.get(0),
        )?;
        Ok(n as u64)
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

    pub fn list_group_members(&self, project: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pubkey, role FROM group_members WHERE project=?1 ORDER BY pubkey")?;
        let rows = stmt.query_map(params![project], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn remove_group_member(&self, project: &str, pubkey: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM group_members WHERE project=?1 AND pubkey=?2",
            params![project, pubkey],
        )?;
        Ok(())
    }

    pub fn list_groups_for_member(&self, pubkey: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT project FROM group_members WHERE pubkey=?1")?;
        let rows = stmt.query_map(params![pubkey], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Return the `(harness_kind, anchor)` pair needed to re-derive a session's
    /// per-session keypair at crash-GC time (Stage 2 / Issue #2).
    ///
    /// - `harness_kind`: the harness label stored in `session_aliases` (e.g.
    ///   "claude-code", "opencode"); falls back to "unknown" when no alias row
    ///   exists for the session.
    /// - `anchor`: the harness-native session id when the harness supplied one
    ///   (`external_id_kind = 'harness'`), otherwise the canonical `session_id`
    ///   itself (which is what opencode / unknown harnesses use as the anchor).
    ///
    /// Reconstruction is correct for all realistic harnesses:
    ///   - claude-code / codex: alias row with kind='harness' present → anchor = native id
    ///   - opencode: only kind='resume' row present → anchor = session_id
    ///   - unknown: possibly no alias rows → ("unknown", session_id)
    pub fn get_session_derivation_anchor(&self, session_id: &str) -> (String, String) {
        let harness_kind: String = self
            .conn
            .query_row(
                "SELECT harness FROM session_aliases WHERE session_id=?1 LIMIT 1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "unknown".to_string());

        let native_id: Option<String> = self
            .conn
            .query_row(
                "SELECT external_id FROM session_aliases
                 WHERE session_id=?1 AND external_id_kind='harness'
                 ORDER BY created_at DESC LIMIT 1",
                params![session_id],
                |r| r.get::<_, String>(0),
            )
            .ok();

        let anchor = native_id.unwrap_or_else(|| session_id.to_string());
        (harness_kind, anchor)
    }

    /// Apply a relay-authoritative 39002 members snapshot for one group: replace
    /// the cached membership wholesale so we self-heal if our optimistic writes drifted.
    /// Kept for back-compat and test use; deletes ALL rows including admins.
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

    /// Apply a relay-authoritative 39002 plain-member snapshot for one group.
    ///
    /// Unlike `replace_group_members`, this only deletes rows where
    /// `role != 'admin'` so that admin rows written by the 39001 materializer
    /// (or by optimistic `upsert_group_member` calls) are preserved. Use this
    /// when handling kind:39002 events, which carry the plain-member list only.
    pub fn replace_group_plain_members(
        &self,
        project: &str,
        members: &[(String, String)],
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM group_members WHERE project=?1 AND role != 'admin'",
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

    /// Apply a relay-authoritative 39001 admin snapshot for one group.
    ///
    /// Deletes only `role='admin'` rows and re-inserts from the provided list,
    /// leaving plain-member rows intact. Use this when handling kind:39001
    /// events, which carry the admin list only.
    pub fn replace_group_admins(
        &self,
        project: &str,
        admins: &[(String, String)],
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM group_members WHERE project=?1 AND role = 'admin'",
            params![project],
        )?;
        for (pubkey, role) in admins {
            self.conn.execute(
                "INSERT INTO group_members (project, pubkey, role, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(project, pubkey) DO UPDATE SET role=?3, updated_at=?4",
                params![project, pubkey, role, ts],
            )?;
        }
        Ok(())
    }

    // ── session pubkeys (Stage 3 / Issue #2) ────────────────────────────────

    /// Record the derived per-session pubkey and its owning session.
    /// Called on session_start immediately after `derive_session_keys`.
    pub fn upsert_session_pubkey(
        &self,
        session_pubkey: &str,
        session_id: &str,
        agent_pubkey: &str,
        agent_slug: &str,
        created_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_pubkeys (session_pubkey, session_id, agent_pubkey, agent_slug, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_pubkey) DO UPDATE SET session_id=?2, agent_pubkey=?3, agent_slug=?4",
            params![session_pubkey, session_id, agent_pubkey, agent_slug, created_at],
        )?;
        Ok(())
    }

    /// Remove all session pubkey rows for a session.
    /// Called on session_end / engine self-exit / crash-GC.
    pub fn remove_session_pubkeys_for_session(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM session_pubkeys WHERE session_id=?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Resolve a session pubkey to its (session_id, agent_pubkey, agent_slug).
    /// Returns `None` when the pubkey is not a known session pubkey.
    /// Used by routing (`route_mention_into_with_id`) and slug resolution.
    pub fn session_pubkey_info(&self, session_pubkey: &str) -> Option<(String, String, String)> {
        self.conn
            .query_row(
                "SELECT session_id, agent_pubkey, agent_slug FROM session_pubkeys WHERE session_pubkey=?1",
                params![session_pubkey],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .ok()
    }

    /// Resolve a session id to its derived session pubkey (reverse of
    /// `session_pubkey_info`). Returns `None` when no session key was derived
    /// (operator nsec absent). Callers fall back to the durable agent pubkey.
    pub fn session_pubkey_for_session(&self, session_id: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT session_pubkey FROM session_pubkeys WHERE session_id=?1 LIMIT 1",
                params![session_id],
                |r| r.get(0),
            )
            .ok()
    }
}
