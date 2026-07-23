//! Pubkey-keyed durable session and runtime-incarnation persistence.

use super::*;
use rusqlite::{Transaction, TransactionBehavior};

pub(super) const COLS: &str =
    "pubkey, runtime_generation, agent_slug, channel_h, work_root, readiness_parent, \
     observed_harness, claimed_harness, admitted_bundle, admitted_transport, \
     endpoint_provenance, child_pid, transcript_path, runtime_state, presentation_state, \
     work_state, recovery_state, lifecycle_epoch, attachment_epoch, idle_since, idle_deadline, \
     stopped_at, stop_reason, turn_count, busy_seconds, created_at, last_seen, turn_started_at, \
     seen_cursor, title, explicit_chat_published_at, state_changed_at";

fn conversion<T>(index: usize, result: Result<T>) -> rusqlite::Result<T> {
    result.map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error.to_string())),
        )
    })
}

pub(super) fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
    let runtime_state = conversion(13, RuntimeState::parse(&row.get::<_, String>(13)?))?;
    let presentation_state = conversion(14, PresentationState::parse(&row.get::<_, String>(14)?))?;
    let work_state = conversion(15, WorkState::parse(&row.get::<_, String>(15)?))?;
    let recovery_state = conversion(16, RecoveryState::parse(&row.get::<_, String>(16)?))?;
    let stop_reason = row
        .get::<_, Option<String>>(22)?
        .map(|value| conversion(22, StopReason::parse(&value)))
        .transpose()?;
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
        runtime_state,
        presentation_state,
        work_state,
        recovery_state,
        lifecycle_epoch: row.get(17)?,
        attachment_epoch: row.get(18)?,
        idle_since: row.get(19)?,
        idle_deadline: row.get(20)?,
        stopped_at: row.get(21)?,
        stop_reason,
        turn_count: row.get(23)?,
        busy_seconds: row.get(24)?,
        created_at: row.get(25)?,
        last_seen: row.get(26)?,
        turn_started_at: row.get(27)?,
        seen_cursor: row.get(28)?,
        title: row.get(29)?,
        explicit_chat_published_at: row.get(30)?,
        state_changed_at: row.get(31)?,
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

    /// Reserve the sole running incarnation together with its admitted facts.
    /// Stopping runtimes still own their pubkey; only stopped runtimes advance
    /// the generation, and revoked recovery authority can never be relaunched.
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
                "SELECT runtime_generation, runtime_state, recovery_state
                 FROM sessions WHERE pubkey=?1",
                [&r.pubkey],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((_, state, recovery)) = previous.as_ref() {
            if RuntimeState::parse(state)? != RuntimeState::Stopped {
                anyhow::bail!("pubkey {} already has an active runtime", r.pubkey);
            }
            if RecoveryState::parse(recovery)? == RecoveryState::Revoked {
                anyhow::bail!("pubkey {} recovery authority is revoked", r.pubkey);
            }
        }
        let generation = match previous {
            Some((generation, _, _)) => generation
                .checked_add(1)
                .context("runtime generation exhausted")?,
            None => 1,
        };
        tx.execute(
            "INSERT INTO sessions
                 (pubkey, runtime_generation, agent_slug, channel_h, observed_harness,
                  claimed_harness, admitted_bundle, admitted_transport, endpoint_provenance,
                  child_pid, transcript_path, runtime_state, presentation_state, work_state,
                  recovery_state, lifecycle_epoch, created_at, last_seen, state_changed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                     'running', 'unavailable', 'idle', 'pending', 1, ?12, ?12, ?12)
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
                 child_pid=excluded.child_pid, transcript_path=excluded.transcript_path,
                 runtime_state='running', presentation_state='unavailable', work_state='idle',
                 lifecycle_epoch=sessions.lifecycle_epoch+1, attachment_epoch=0,
                 idle_since=0, idle_deadline=0, stopped_at=0, stop_reason=NULL,
                 created_at=excluded.created_at, last_seen=excluded.last_seen, turn_started_at=0,
                 state_changed_at=excluded.state_changed_at",
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
            grant_route_and_initialize_standing(&tx, r)?;
        }
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

    pub fn list_running_sessions(&self) -> Result<Vec<Session>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLS} FROM sessions WHERE runtime_state='running' ORDER BY created_at DESC"
        ))?;
        let rows = statement.query_map([], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_stopping_sessions(&self) -> Result<Vec<Session>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLS} FROM sessions WHERE runtime_state='stopping' ORDER BY pubkey"
        ))?;
        let rows = statement.query_map([], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn session_can_fresh_relaunch_exact(&self, pubkey: &str) -> Result<bool> {
        Ok(self
            .get_session(pubkey)?
            .is_some_and(|session| session.can_fresh_relaunch_exact()))
    }
    pub fn touch_session(&self, pubkey: &str, last_seen: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_seen=?2 WHERE pubkey=?1",
            params![pubkey, last_seen],
        )?;
        self.touch_handle_for_pubkey(pubkey, last_seen)
    }
    pub fn bind_runtime_process(
        &self,
        pubkey: &str,
        runtime_generation: u64,
        child_pid: Option<i32>,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE sessions SET child_pid=?3, last_seen=?4
             WHERE pubkey=?1 AND runtime_generation=?2 AND runtime_state='running'
               AND recovery_state<>'revoked'",
            params![
                pubkey,
                runtime_generation,
                child_pid,
                crate::util::now_secs()
            ],
        )? > 0)
    }

    pub fn mark_runtime_stopped_if_generation(
        &self,
        pubkey: &str,
        generation: u64,
        reason: StopReason,
        stopped_at: u64,
    ) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE sessions
             SET runtime_state='stopped', presentation_state='unavailable', work_state='idle',
                 lifecycle_epoch=lifecycle_epoch+1, idle_since=0, idle_deadline=0,
                 busy_seconds=busy_seconds + CASE WHEN work_state='working'
                     AND turn_started_at>0 THEN MAX(0, ?4-turn_started_at) ELSE 0 END,
                 stopped_at=?4, stop_reason=?3, turn_started_at=0, state_changed_at=?4
             WHERE pubkey=?1 AND runtime_generation=?2 AND runtime_state='running'",
            params![pubkey, generation, reason.as_str(), stopped_at],
        )?;
        if changed > 0 {
            mark_handle_stopped(&tx, pubkey)?;
            let lifecycle_epoch: u64 = tx.query_row(
                "SELECT lifecycle_epoch FROM sessions WHERE pubkey=?1",
                [pubkey],
                |row| row.get(0),
            )?;
            let retain_until =
                if matches!(reason, StopReason::AttachedCleanExit | StopReason::Revoked) {
                    stopped_at
                } else {
                    stopped_at.saturating_add(STOPPED_STANDING_RETENTION_SECS)
                };
            super::session_standing::retain_in_transaction(
                &tx,
                pubkey,
                lifecycle_epoch,
                retain_until,
                stopped_at,
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }
    pub fn mark_runtime_stopped(
        &self,
        pubkey: &str,
        reason: StopReason,
        stopped_at: u64,
    ) -> Result<bool> {
        match self
            .get_session(pubkey)?
            .map(|session| session.runtime_generation)
        {
            Some(generation) => {
                self.mark_runtime_stopped_if_generation(pubkey, generation, reason, stopped_at)
            }
            None => Ok(false),
        }
    }
}

fn grant_route_and_initialize_standing(
    tx: &Transaction<'_>,
    registration: &RegisterSession,
) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO session_channels (pubkey, channel_h, granted_at)
         VALUES (?1, ?2, ?3)",
        params![
            registration.pubkey,
            registration.channel_h,
            registration.now
        ],
    )?;
    tx.execute(
        "INSERT INTO session_standing
             (pubkey, channel_h, state, retain_until, standing_epoch,
              session_lifecycle_epoch, updated_at)
         SELECT ?1, ?2, 'absent', 0, 1, lifecycle_epoch, ?3
         FROM sessions WHERE pubkey=?1
         ON CONFLICT(pubkey, channel_h) DO NOTHING",
        params![
            registration.pubkey,
            registration.channel_h,
            registration.now
        ],
    )?;
    Ok(())
}

fn mark_handle_stopped(tx: &Transaction<'_>, pubkey: &str) -> Result<()> {
    tx.execute(
        "UPDATE handle_leases SET live=0,
             last_active_at=MAX(last_active_at,
                 COALESCE((SELECT last_seen FROM sessions WHERE pubkey=?1), 0))
         WHERE pubkey=?1",
        [pubkey],
    )?;
    Ok(())
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
