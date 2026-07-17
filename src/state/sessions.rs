//! Pubkey-keyed local runtime persistence.

use super::*;
use rusqlite::{Transaction, TransactionBehavior};

pub(super) const COLS: &str =
    "pubkey, runtime_generation, agent_slug, channel_h, work_root, readiness_parent, \
     observed_harness, claimed_harness, admitted_bundle, admitted_transport, \
     endpoint_provenance, child_pid, transcript_path, alive, created_at, last_seen, working, \
     turn_started_at, seen_cursor, title, explicit_chat_published_at";

pub(super) fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
    Ok(Session {
        pubkey: row.get(0)?,
        runtime_generation: row.get(1)?,
        agent_slug: row.get(2)?,
        channel_h: row.get(3)?,
        work_root: row.get(4)?,
        readiness_parent: row.get(5)?,
        observed_harness: row.get(6)?,
        claimed_harness: row.get(7)?,
        admitted_bundle: row.get(8)?,
        admitted_transport: row.get(9)?,
        endpoint_provenance: row.get(10)?,
        child_pid: row.get(11)?,
        transcript_path: row.get(12)?,
        alive: row.get::<_, i64>(13)? != 0,
        created_at: row.get(14)?,
        last_seen: row.get(15)?,
        working: row.get::<_, i64>(16)? != 0,
        turn_started_at: row.get(17)?,
        seen_cursor: row.get(18)?,
        title: row.get(19)?,
        explicit_chat_published_at: row.get(20)?,
    })
}

impl Store {
    #[cfg(test)]
    pub(crate) fn reserve_hook_session_for_test(&self, r: &RegisterSession) -> Result<u64> {
        let observed = r.observed_harness.clone();
        self.reserve_session_with_facts(
            r,
            &AdmittedRuntimeFacts {
                observed_harness: observed.clone(),
                claimed_harness: observed,
                bundle: String::new(),
                transport: String::new(),
                endpoint_provenance: "hook".to_string(),
            },
        )
    }

    /// Atomically reserve one runtime together with its complete admitted facts.
    /// A dead runtime may be replaced; its monotonically increasing generation
    /// fences late completion from the previous incarnation.
    pub fn reserve_session_with_facts(
        &self,
        r: &RegisterSession,
        facts: &AdmittedRuntimeFacts,
    ) -> Result<u64> {
        if r.pubkey.trim().is_empty() {
            anyhow::bail!("session pubkey must not be empty");
        }
        validate_runtime_facts(r, facts)?;
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let previous = tx
            .query_row(
                "SELECT runtime_generation, alive FROM sessions WHERE pubkey=?1",
                [&r.pubkey],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?;
        if previous.is_some_and(|(_, alive)| alive) {
            anyhow::bail!("pubkey {} already has an active runtime", r.pubkey);
        }
        let generation = match previous {
            Some((generation, _)) => generation
                .checked_add(1)
                .context("runtime generation exhausted")?,
            None => 1,
        };
        tx.execute(
            "INSERT INTO sessions
                 (pubkey, runtime_generation, agent_slug, channel_h, observed_harness,
                  claimed_harness, admitted_bundle, admitted_transport, endpoint_provenance,
                  child_pid, transcript_path, alive, created_at, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12)
             ON CONFLICT(pubkey) DO UPDATE SET
                 runtime_generation=excluded.runtime_generation,
                 agent_slug=excluded.agent_slug, channel_h=excluded.channel_h,
                 observed_harness=CASE
                     WHEN excluded.endpoint_provenance='launch' THEN excluded.observed_harness
                     WHEN sessions.endpoint_provenance='launch' THEN sessions.observed_harness
                     ELSE excluded.observed_harness END,
                 claimed_harness=CASE WHEN excluded.claimed_harness<>''
                     THEN excluded.claimed_harness ELSE sessions.claimed_harness END,
                 admitted_bundle=CASE
                     WHEN excluded.endpoint_provenance='launch' THEN excluded.admitted_bundle
                     WHEN sessions.endpoint_provenance='launch' THEN sessions.admitted_bundle
                     ELSE excluded.admitted_bundle END,
                 admitted_transport=CASE
                     WHEN excluded.endpoint_provenance='launch' THEN excluded.admitted_transport
                     WHEN sessions.endpoint_provenance='launch' THEN sessions.admitted_transport
                     ELSE excluded.admitted_transport END,
                 endpoint_provenance=CASE
                     WHEN excluded.endpoint_provenance='launch' THEN excluded.endpoint_provenance
                     WHEN sessions.endpoint_provenance='launch' THEN sessions.endpoint_provenance
                     ELSE excluded.endpoint_provenance END,
                 child_pid=excluded.child_pid,
                 transcript_path=excluded.transcript_path, alive=1,
                 created_at=excluded.created_at, last_seen=excluded.last_seen,
                 working=0, turn_started_at=0",
            params![
                r.pubkey,
                generation,
                r.agent_slug,
                r.channel_h,
                facts.observed_harness,
                facts.claimed_harness,
                facts.bundle,
                facts.transport,
                facts.endpoint_provenance,
                r.child_pid,
                r.transcript_path,
                r.now,
            ],
        )?;
        if !r.channel_h.trim().is_empty() {
            tx.execute(
                "INSERT OR IGNORE INTO session_channels (pubkey, channel_h, joined_at)
                 VALUES (?1, ?2, ?3)",
                params![r.pubkey, r.channel_h, r.now],
            )?;
        }
        tx.execute(
            "DELETE FROM session_claims WHERE pubkey=?1 AND channel_h=?2",
            params![r.pubkey, r.channel_h],
        )?;
        tx.execute(
            "UPDATE handle_leases SET live=1, last_active_at=?2 WHERE pubkey=?1",
            params![r.pubkey, r.now],
        )?;
        tx.commit()?;
        Ok(generation)
    }

    /// Record a hook host claim without changing launch/process-observed facts.
    pub fn record_claimed_harness(&self, pubkey: &str, claimed_harness: &str) -> Result<()> {
        if claimed_harness.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "UPDATE sessions SET claimed_harness=?2 WHERE pubkey=?1",
            params![pubkey, claimed_harness],
        )?;
        Ok(())
    }

    pub fn get_session(&self, pubkey: &str) -> Result<Option<Session>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM sessions WHERE pubkey=?1"),
                [pubkey],
                row_to_session,
            )
            .optional()?)
    }

    pub(crate) fn session_exists(&self, pubkey: &str) -> Result<bool> {
        Ok(self.get_session(pubkey)?.is_some())
    }

    pub fn list_alive_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM sessions WHERE alive=1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt.query_map([], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_working(&self, pubkey: &str, working: bool, turn_started_at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET working=?2, turn_started_at=?3 WHERE pubkey=?1",
            params![pubkey, working as i64, turn_started_at],
        )?;
        Ok(())
    }

    pub fn touch_session(&self, pubkey: &str, last_seen: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_seen=?2 WHERE pubkey=?1",
            params![pubkey, last_seen],
        )?;
        self.touch_handle_for_pubkey(pubkey, last_seen)
    }

    pub fn set_session_transcript(&self, pubkey: &str, transcript_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET transcript_path=?2 WHERE pubkey=?1",
            params![pubkey, transcript_path],
        )?;
        Ok(())
    }

    /// Attach the host process to exactly the runtime incarnation reserved
    /// before spawn. A stale bootstrap cannot overwrite a newer incarnation.
    pub fn bind_runtime_process(
        &self,
        pubkey: &str,
        runtime_generation: u64,
        child_pid: Option<i32>,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE sessions SET child_pid=?3, last_seen=?4
             WHERE pubkey=?1 AND runtime_generation=?2 AND alive=1",
            params![
                pubkey,
                runtime_generation,
                child_pid,
                crate::util::now_secs()
            ],
        )? > 0)
    }

    pub fn set_session_channel(&self, pubkey: &str, channel_h: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET channel_h=?2 WHERE pubkey=?1",
            params![pubkey, channel_h],
        )?;
        if !channel_h.trim().is_empty() {
            self.join_session_channel(pubkey, channel_h, crate::util::now_secs())?;
        }
        Ok(())
    }

    pub fn set_session_context(
        &self,
        pubkey: &str,
        channel_h: &str,
        work_root: &str,
        readiness_parent: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions
             SET channel_h=?2, work_root=?3, readiness_parent=?4
             WHERE pubkey=?1",
            params![pubkey, channel_h, work_root, readiness_parent],
        )?;
        if !channel_h.trim().is_empty() {
            self.join_session_channel(pubkey, channel_h, crate::util::now_secs())?;
        }
        Ok(())
    }

    /// Admission-time immediate parent for a channel whose relay metadata may
    /// not have materialized yet. This is host context, not channel truth.
    pub fn session_readiness_parent(&self, channel_h: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT readiness_parent FROM sessions
                 WHERE channel_h=?1 AND readiness_parent<>''
                 ORDER BY alive DESC, created_at DESC LIMIT 1",
                [channel_h],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn join_session_channel(
        &self,
        pubkey: &str,
        channel_h: &str,
        joined_at: u64,
    ) -> Result<()> {
        if channel_h.trim().is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO session_channels (pubkey, channel_h, joined_at)
             VALUES (?1, ?2, ?3)",
            params![pubkey, channel_h, joined_at],
        )?;
        Ok(())
    }

    pub fn leave_session_channel(&self, pubkey: &str, channel_h: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM session_channels WHERE pubkey=?1 AND channel_h=?2",
            params![pubkey, channel_h],
        )? > 0)
    }

    pub fn is_session_joined_channel(&self, pubkey: &str, channel_h: &str) -> Result<bool> {
        if self
            .get_session(pubkey)?
            .is_some_and(|session| session.channel_h == channel_h)
        {
            return Ok(true);
        }
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_channels WHERE pubkey=?1 AND channel_h=?2)",
            params![pubkey, channel_h],
            |row| row.get(0),
        )?)
    }

    pub fn list_session_joined_channels(&self, pubkey: &str) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT channel_h, joined_at FROM session_channels
             WHERE pubkey=?1 ORDER BY joined_at ASC, channel_h ASC",
        )?;
        let rows = stmt.query_map([pubkey], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut joined = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if let Some(session) = self.get_session(pubkey)? {
            if !session.channel_h.is_empty() && !joined.iter().any(|(h, _)| h == &session.channel_h)
            {
                joined.push((session.channel_h, session.created_at));
            }
        }
        joined.sort_by(|(a_h, a_t), (b_h, b_t)| a_t.cmp(b_t).then(a_h.cmp(b_h)));
        Ok(joined)
    }

    /// End only the incarnation the caller started. Returns false when a newer
    /// generation is already active.
    pub fn mark_dead_if_generation(&self, pubkey: &str, generation: u64) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE sessions SET alive=0, working=0
             WHERE pubkey=?1 AND runtime_generation=?2 AND alive=1",
            params![pubkey, generation],
        )?;
        if changed > 0 {
            tx.execute(
                "UPDATE handle_leases SET live=0,
                     last_active_at=MAX(last_active_at,
                         COALESCE((SELECT last_seen FROM sessions WHERE pubkey=?1), 0))
                 WHERE pubkey=?1",
                [pubkey],
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    pub fn mark_dead(&self, pubkey: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET alive=0, working=0 WHERE pubkey=?1",
            [pubkey],
        )?;
        self.mark_handle_offline_for_pubkey(pubkey)
    }
}

fn validate_runtime_facts(r: &RegisterSession, facts: &AdmittedRuntimeFacts) -> Result<()> {
    let observed = facts.observed_harness.trim();
    if observed.is_empty() {
        anyhow::bail!("runtime facts require observed_harness");
    }
    let harness = crate::session::Harness::from_str(observed);
    if harness == crate::session::Harness::Unknown || harness.as_str() != observed {
        anyhow::bail!("runtime facts contain unknown observed_harness {observed:?}");
    }
    if r.observed_harness != observed {
        anyhow::bail!(
            "registration observed_harness {:?} does not match admitted facts {observed:?}",
            r.observed_harness
        );
    }
    if !matches!(facts.transport.as_str(), "" | "pty" | "acp" | "app-server") {
        anyhow::bail!(
            "runtime facts contain unknown transport {:?}",
            facts.transport
        );
    }
    match facts.endpoint_provenance.as_str() {
        "launch" => {
            if !facts.claimed_harness.is_empty() {
                anyhow::bail!("launch runtime facts forbid claimed_harness");
            }
            if facts.bundle.trim().is_empty() {
                anyhow::bail!("launch runtime facts require bundle");
            }
            if facts.transport.is_empty() {
                anyhow::bail!("launch runtime facts require transport");
            }
        }
        "hook" => {
            let claimed = facts.claimed_harness.trim();
            if claimed.is_empty() {
                anyhow::bail!("hook runtime facts require claimed_harness");
            }
            let claimed_harness = crate::session::Harness::from_str(claimed);
            if claimed_harness == crate::session::Harness::Unknown
                || claimed_harness.as_str() != claimed
            {
                anyhow::bail!("runtime facts contain unknown claimed_harness {claimed:?}");
            }
            if !facts.bundle.is_empty() {
                anyhow::bail!("hook runtime facts forbid bundle");
            }
        }
        provenance => anyhow::bail!(
            "runtime facts require endpoint_provenance launch or hook, got {provenance:?}"
        ),
    }
    Ok(())
}
