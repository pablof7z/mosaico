use super::Store;
use crate::domain::Lifecycle;
use crate::session::{
    PeerStatusObservation, SessionId, SessionSnapshot, SnapshotSource, TitleSource,
};
use anyhow::Result;
use rusqlite::params;

const PRESENCE_COLS: &str = "pubkey, project, local_session_id, agent_slug, host, rel_cwd, \
     title, title_source, activity, busy, phase, turn_id, turn_started_at, last_distill_at, \
     last_seen, resume_id, state_version, lifecycle, first_seen, updated_at";

impl Store {
    pub(super) fn presence_snapshots(
        &self,
        project: Option<&str>,
        since: u64,
    ) -> Result<Vec<SessionSnapshot>> {
        let sql = format!(
            "SELECT {PRESENCE_COLS} FROM presence_state
             WHERE last_seen>=?1 AND (?2 IS NULL OR project=?2)
             ORDER BY last_seen DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, project], row_to_presence_state)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub(super) fn presence_delta_snapshots(
        &self,
        project: &str,
        since: u64,
        now_minus_ttl: u64,
        ttl: u64,
    ) -> Result<Vec<SessionSnapshot>> {
        let sql = format!(
            "SELECT {PRESENCE_COLS} FROM presence_state
             WHERE project=?1
               AND (first_seen>=?2 OR updated_at>=?2 OR (last_seen < ?3 AND last_seen+?4 >= ?2))"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![project, since, now_minus_ttl, ttl],
            row_to_presence_state,
        )?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn confirm_local_presence(
        &self,
        snap: &SessionSnapshot,
        signer_pubkey: &str,
        native_event_id: &str,
        confirmed_at: u64,
    ) -> Result<()> {
        let local_session_id = snap.session_id.as_str();
        self.upsert_presence(
            signer_pubkey,
            &snap.project,
            local_session_id,
            &snap.agent_slug,
            &snap.host,
            &snap.rel_cwd,
            &snap.title,
            snap.title_source.as_str(),
            &snap.activity,
            snap.busy,
            &snap.phase,
            snap.turn_id,
            snap.turn_started_at,
            snap.last_distill_at,
            snap.last_seen,
            &snap.resume_id,
            snap.state_version,
            snap.lifecycle.as_str(),
            snap.first_seen,
            snap.updated_at,
            native_event_id,
            confirmed_at,
        )
    }

    pub(super) fn record_relay_presence(
        &self,
        obs: &PeerStatusObservation,
        native_event_id: Option<&str>,
    ) -> Result<()> {
        let existing = self.presence_existing(&obs.agent_pubkey, &obs.project)?;
        let busy_i = obs.busy as i64;
        let (version, updated_at, first_seen) = match existing {
            None => (1, obs.observed_at, obs.emitted_at),
            Some(existing) => {
                let content_changed = existing.title != obs.title
                    || existing.activity != obs.activity
                    || existing.busy != busy_i
                    || existing.host != obs.host
                    || existing.rel_cwd != obs.rel_cwd
                    || (!obs.agent_slug.is_empty() && existing.agent_slug != obs.agent_slug);
                (
                    if content_changed {
                        existing.state_version + 1
                    } else {
                        existing.state_version
                    },
                    if content_changed {
                        obs.observed_at
                    } else {
                        existing.updated_at
                    },
                    existing.first_seen,
                )
            }
        };
        self.upsert_presence(
            &obs.agent_pubkey,
            &obs.project,
            "",
            &obs.agent_slug,
            &obs.host,
            &obs.rel_cwd,
            &obs.title,
            TitleSource::Peer.as_str(),
            &obs.activity,
            obs.busy,
            if obs.busy { "working" } else { "idle" },
            0,
            0,
            0,
            obs.emitted_at,
            "",
            version,
            Lifecycle::Active.as_str(),
            first_seen,
            updated_at,
            native_event_id.unwrap_or(""),
            obs.observed_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn upsert_presence(
        &self,
        pubkey: &str,
        project: &str,
        local_session_id: &str,
        agent_slug: &str,
        host: &str,
        rel_cwd: &str,
        title: &str,
        title_source: &str,
        activity: &str,
        busy: bool,
        phase: &str,
        turn_id: i64,
        turn_started_at: u64,
        last_distill_at: u64,
        last_seen: u64,
        resume_id: &str,
        state_version: i64,
        lifecycle: &str,
        first_seen: u64,
        updated_at: u64,
        native_event_id: &str,
        confirmed_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO presence_state
               (pubkey, project, local_session_id, agent_slug, host, rel_cwd,
                title, title_source, activity, busy, phase, turn_id, turn_started_at,
                last_distill_at, last_seen, resume_id, state_version, lifecycle,
                first_seen, updated_at, native_event_id, confirmed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)
             ON CONFLICT(pubkey, project) DO UPDATE SET
               local_session_id=CASE WHEN excluded.local_session_id<>'' THEN excluded.local_session_id ELSE presence_state.local_session_id END,
               agent_slug=CASE WHEN excluded.agent_slug<>'' THEN excluded.agent_slug ELSE presence_state.agent_slug END,
               host=excluded.host,
               rel_cwd=excluded.rel_cwd,
               title=excluded.title,
               title_source=excluded.title_source,
               activity=excluded.activity,
               busy=excluded.busy,
               phase=excluded.phase,
               turn_id=excluded.turn_id,
               turn_started_at=excluded.turn_started_at,
               last_distill_at=excluded.last_distill_at,
               last_seen=MAX(presence_state.last_seen, excluded.last_seen),
               resume_id=excluded.resume_id,
               state_version=excluded.state_version,
               lifecycle=excluded.lifecycle,
               updated_at=excluded.updated_at,
               native_event_id=CASE WHEN excluded.native_event_id<>'' THEN excluded.native_event_id ELSE presence_state.native_event_id END,
               confirmed_at=excluded.confirmed_at",
            params![
                pubkey,
                project,
                local_session_id,
                agent_slug,
                host,
                rel_cwd,
                title,
                title_source,
                activity,
                busy as i64,
                phase,
                turn_id,
                turn_started_at,
                last_distill_at,
                last_seen,
                resume_id,
                state_version,
                lifecycle,
                first_seen,
                updated_at,
                native_event_id,
                confirmed_at,
            ],
        )?;
        Ok(())
    }

    fn presence_existing(&self, pubkey: &str, project: &str) -> Result<Option<PresenceExisting>> {
        Ok(self
            .conn
            .query_row(
                "SELECT title, activity, busy, host, rel_cwd, agent_slug, state_version, first_seen, updated_at
                 FROM presence_state WHERE pubkey=?1 AND project=?2",
                params![pubkey, project],
                |r| {
                    Ok(PresenceExisting {
                        title: r.get(0)?,
                        activity: r.get(1)?,
                        busy: r.get(2)?,
                        host: r.get(3)?,
                        rel_cwd: r.get(4)?,
                        agent_slug: r.get(5)?,
                        state_version: r.get(6)?,
                        first_seen: r.get(7)?,
                        updated_at: r.get(8)?,
                    })
                },
            )
            .ok())
    }
}

struct PresenceExisting {
    title: String,
    activity: String,
    busy: i64,
    host: String,
    rel_cwd: String,
    agent_slug: String,
    state_version: i64,
    first_seen: u64,
    updated_at: u64,
}

fn row_to_presence_state(row: &rusqlite::Row) -> rusqlite::Result<SessionSnapshot> {
    let pubkey: String = row.get(0)?;
    let local_session_id: String = row.get(2)?;
    let session_id = if local_session_id.is_empty() {
        pubkey.clone()
    } else {
        local_session_id
    };
    Ok(SessionSnapshot {
        source: SnapshotSource::Peer,
        agent_pubkey: pubkey,
        project: row.get(1)?,
        session_id: SessionId::from(session_id),
        agent_slug: row.get(3)?,
        host: row.get(4)?,
        rel_cwd: row.get(5)?,
        title: row.get(6)?,
        title_source: TitleSource::from_str(&row.get::<_, String>(7)?),
        activity: row.get(8)?,
        busy: row.get::<_, i64>(9)? != 0,
        phase: row.get(10)?,
        turn_id: row.get(11)?,
        turn_started_at: row.get(12)?,
        last_distill_at: row.get(13)?,
        last_seen: row.get(14)?,
        resume_id: row.get(15)?,
        state_version: row.get(16)?,
        lifecycle: Lifecycle::from_str(&row.get::<_, String>(17)?),
        first_seen: row.get(18)?,
        updated_at: row.get(19)?,
    })
}
