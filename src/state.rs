//! Local app state in SQLite (M1 §2, §7).
//!
//! NMP-shaped event stores aside, tenex-edge keeps the *app* state the fabric
//! shouldn't own: my own sessions (+ the CC pid to watch), a directory of peers
//! built from their profiles/presence, and a per-session inbox of mentions —
//! idempotent on `(mention_event_id, target_session)` so the same mention seen
//! by two of an agent's processes injects once per session.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub session_id: String,
    pub agent_slug: String,
    pub agent_pubkey: String,
    pub project: String,
    pub host: String,
    pub child_pid: Option<i32>,
    pub watch_pid: Option<i32>,
    pub created_at: u64,
    pub alive: bool,
    /// Project-relative working directory advertised on presence/status.
    pub rel_cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSession {
    pub session_id: String,
    pub pubkey: String,
    pub slug: String,
    pub project: String,
    pub host: String,
    pub last_seen: u64,
    /// Peer's project-relative working dir, learned from its presence events.
    pub rel_cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAgent {
    pub pubkey: String,
    pub slug: String,
    pub host: String,
    pub owners: String, // comma-joined owner pubkeys
    pub first_seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxRow {
    pub mention_event_id: String,
    pub target_session: String,
    pub from_pubkey: String,
    pub from_slug: String,
    pub project: String,
    pub body: String,
    pub created_at: u64,
    /// The sender's session id (empty when unknown — old peers / untargeted).
    /// Lets the recipient reply to the exact sibling session that wrote this.
    pub from_session: String,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    session_id    TEXT PRIMARY KEY,
    agent_slug    TEXT NOT NULL,
    agent_pubkey  TEXT NOT NULL,
    project       TEXT NOT NULL,
    host          TEXT NOT NULL,
    child_pid     INTEGER,
    watch_pid     INTEGER,
    created_at    INTEGER NOT NULL,
    last_seen     INTEGER NOT NULL DEFAULT 0,
    transcript_path TEXT,
    alive         INTEGER NOT NULL DEFAULT 1,
    rel_cwd       TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS profiles (
    pubkey     TEXT PRIMARY KEY,
    slug       TEXT NOT NULL,
    host       TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS peer_sessions (
    session_id TEXT PRIMARY KEY,
    pubkey     TEXT NOT NULL,
    slug       TEXT NOT NULL,
    project    TEXT NOT NULL,
    host       TEXT NOT NULL,
    last_seen  INTEGER NOT NULL,
    first_seen INTEGER NOT NULL DEFAULT 0,
    rel_cwd    TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS inbox (
    mention_event_id TEXT NOT NULL,
    target_session   TEXT NOT NULL,
    from_pubkey      TEXT NOT NULL,
    from_slug        TEXT NOT NULL,
    project          TEXT NOT NULL,
    body             TEXT NOT NULL,
    created_at       INTEGER NOT NULL,
    delivered        INTEGER NOT NULL DEFAULT 0,
    from_session     TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (mention_event_id, target_session)
);
-- Per-session turn state: flipped by the host's turn-start/turn-end hooks. The
-- engine polls this to decide when to distill activity (30s into a turn, then
-- every few minutes) and when to go idle. No tool events — distillation reads
-- the conversation transcript, not tool names.
CREATE TABLE IF NOT EXISTS turn_state (
    session_id      TEXT PRIMARY KEY,
    working         INTEGER NOT NULL DEFAULT 0,
    turn_started_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS pending_agents (
    pubkey     TEXT PRIMARY KEY,
    slug       TEXT NOT NULL,
    host       TEXT NOT NULL,
    owners     TEXT NOT NULL,
    first_seen INTEGER NOT NULL
);
-- A mention an agent has already received, so it is never re-delivered in a
-- later session (mentions are stored kind:1 events that persist on the relay).
CREATE TABLE IF NOT EXISTS seen_mentions (
    agent_pubkey     TEXT NOT NULL,
    mention_event_id TEXT NOT NULL,
    seen_at          INTEGER NOT NULL,
    PRIMARY KEY (agent_pubkey, mention_event_id)
);
-- Current "what each agent is doing" (NIP-38 status), per (agent, project).
CREATE TABLE IF NOT EXISTS agent_status (
    pubkey     TEXT NOT NULL,
    project    TEXT NOT NULL,
    text       TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (pubkey, project)
);
-- NIP-29 group metadata cache: the 'about' text for each project channel (kind 39000).
CREATE TABLE IF NOT EXISTS project_meta (
    project    TEXT PRIMARY KEY,
    about      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
-- NIP-29 groups this daemon owns/manages (created + locked closed via userNsec).
CREATE TABLE IF NOT EXISTS owned_groups (
    project    TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL
);
-- NIP-29 group membership cache (relay-authoritative kind 39002 + our optimistic
-- put-user writes). Lets session_start skip redundant 9000 publishes idempotently.
CREATE TABLE IF NOT EXISTS group_members (
    project    TEXT NOT NULL,
    pubkey     TEXT NOT NULL,
    role       TEXT NOT NULL DEFAULT 'member',
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project, pubkey)
);
"#;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        // WAL stopgap (M1 daemon migration, stage 1): until the per-machine daemon
        // is the sole writer, many processes (per-session engines + every CLI
        // invocation) share this file. WAL + a busy timeout + relaxed sync is the
        // bandage that stops the multi-writer corruption we recovered from. It
        // stays harmless (and a touch faster) once the daemon owns the db.
        //   journal_mode=WAL   readers don't block the writer; one writer at a time
        //   busy_timeout=5000  block up to 5s on a held lock instead of erroring
        //   synchronous=NORMAL safe under WAL; fsync only at checkpoints
        // No foreign_keys pragma: the schema declares no FK constraints.
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();
        conn.busy_timeout(std::time::Duration::from_secs(5)).ok();
        conn.execute_batch(SCHEMA).context("creating schema")?;
        // Migrations (ignore if column already present).
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN last_seen INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN transcript_path TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE peer_sessions ADD COLUMN first_seen INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // §8e: project-relative cwd, on own sessions and the peer directory.
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN rel_cwd TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE peer_sessions ADD COLUMN rel_cwd TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Sender session id on inbox mentions, so a reply can target the exact
        // sibling session that wrote the message.
        let _ = conn.execute(
            "ALTER TABLE inbox ADD COLUMN from_session TEXT NOT NULL DEFAULT ''",
            [],
        );
        Ok(Self { conn })
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// `PRAGMA integrity_check` → "ok" on a healthy db, else the first problem
    /// line. Used by the concurrency/corruption test to assert no corruption.
    pub fn integrity_check(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?)
    }

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

    // ── peer directory ───────────────────────────────────────────────────

    pub fn upsert_profile(&self, pubkey: &str, slug: &str, host: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO profiles (pubkey, slug, host, updated_at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(pubkey) DO UPDATE SET slug=?2, host=?3, updated_at=?4",
            params![pubkey, slug, host, ts],
        )?;
        Ok(())
    }

    pub fn upsert_peer_session(
        &self,
        session_id: &str,
        pubkey: &str,
        slug: &str,
        project: &str,
        host: &str,
        rel_cwd: &str,
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO peer_sessions (session_id, pubkey, slug, project, host, rel_cwd, last_seen, first_seen)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?7)
             ON CONFLICT(session_id) DO UPDATE SET pubkey=?2, slug=?3, project=?4, host=?5, rel_cwd=?6, last_seen=?7",
            params![session_id, pubkey, slug, project, host, rel_cwd, ts],
        )?;
        Ok(())
    }

    /// Resolve an agent slug to a pubkey. With a project scope, this behaves
    /// like `slug@project`: prefer presence in that project, and do not let a
    /// global profile from another project hijack the route.
    pub fn resolve_agent_pubkey(
        &self,
        slug: &str,
        project: Option<&str>,
    ) -> Result<Option<String>> {
        if let Some(project) = project {
            return Ok(self
                .conn
                .query_row(
                    "SELECT pubkey FROM peer_sessions WHERE slug=?1 AND project=?2 ORDER BY last_seen DESC LIMIT 1",
                    params![slug, project],
                    |r| r.get::<_, String>(0),
                )
                .ok());
        }

        if let Some(pk) = self
            .conn
            .query_row(
                "SELECT pubkey FROM profiles WHERE slug=?1 ORDER BY updated_at DESC LIMIT 1",
                params![slug],
                |r| r.get::<_, String>(0),
            )
            .ok()
        {
            return Ok(Some(pk));
        }
        Ok(self
            .conn
            .query_row(
                "SELECT pubkey FROM peer_sessions WHERE slug=?1 ORDER BY last_seen DESC LIMIT 1",
                params![slug],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    /// Find one of MY sessions by session-id prefix (for messaging a sibling
    /// session of the same agent on this machine).
    pub fn find_session_by_prefix(&self, prefix: &str) -> Result<Option<SessionRecord>> {
        let pat = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd
             FROM sessions WHERE session_id LIKE ?1 ORDER BY created_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![pat])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_session(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn find_peer_session_by_prefix(&self, prefix: &str) -> Result<Option<PeerSession>> {
        let pat = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT session_id, pubkey, slug, project, host, last_seen, rel_cwd
             FROM peer_sessions WHERE session_id LIKE ?1 ORDER BY last_seen DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![pat])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_peer(row)?))
        } else {
            Ok(None)
        }
    }

    /// Peer sessions seen at or after `since` (freshness filter). `project=None`
    /// = all projects. A peer is "live" only while its heartbeat keeps `last_seen`
    /// fresh; once heartbeats stop it ages out and is no longer shown.
    pub fn list_peer_sessions(
        &self,
        project: Option<&str>,
        since: u64,
    ) -> Result<Vec<PeerSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, pubkey, slug, project, host, last_seen, rel_cwd FROM peer_sessions
             WHERE last_seen>=?1 AND (?2 IS NULL OR project=?2) ORDER BY last_seen DESC",
        )?;
        let rows: Vec<PeerSession> = stmt
            .query_map(params![since, project], row_to_peer)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Delete peer sessions not seen since `before` (housekeeping for pollution).
    pub fn prune_peer_sessions(&self, before: u64) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM peer_sessions WHERE last_seen<?1",
            params![before],
        )?)
    }

    // ── ACL: pending agents (kind:0 claiming us, not yet authorized) ──────

    pub fn upsert_pending_agent(
        &self,
        pubkey: &str,
        slug: &str,
        host: &str,
        owners: &str,
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pending_agents (pubkey, slug, host, owners, first_seen) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(pubkey) DO UPDATE SET slug=?2, host=?3, owners=?4",
            params![pubkey, slug, host, owners, ts],
        )?;
        Ok(())
    }

    pub fn remove_pending_agent(&self, pubkey: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM pending_agents WHERE pubkey=?1",
            params![pubkey],
        )?;
        Ok(())
    }

    pub fn list_pending_agents(&self) -> Result<Vec<PendingAgent>> {
        let mut stmt = self.conn.prepare(
            "SELECT pubkey, slug, host, owners, first_seen FROM pending_agents ORDER BY first_seen",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PendingAgent {
                    pubkey: row.get(0)?,
                    slug: row.get(1)?,
                    host: row.get(2)?,
                    owners: row.get(3)?,
                    first_seen: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

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
             ), ast.project, ast.text
             FROM agent_status ast
             WHERE ast.updated_at>=?1 AND (?2 IS NULL OR ast.project=?2)
             ORDER BY ast.updated_at",
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

    // ── agent status ("what is X doing") ─────────────────────────────────

    pub fn set_agent_status(&self, pubkey: &str, project: &str, text: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agent_status (pubkey, project, text, updated_at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(pubkey, project) DO UPDATE SET text=?3, updated_at=?4",
            params![pubkey, project, text, ts],
        )?;
        Ok(())
    }

    pub fn get_agent_status(&self, pubkey: &str, project: &str) -> Result<Option<String>> {
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
        let mut stmt = self.conn.prepare(
            "SELECT project, about FROM project_meta ORDER BY project",
        )?;
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

    pub fn upsert_group_member(&self, project: &str, pubkey: &str, role: &str, ts: u64) -> Result<()> {
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
    pub fn replace_group_members(&self, project: &str, members: &[(String, String)], ts: u64) -> Result<()> {
        self.conn
            .execute("DELETE FROM group_members WHERE project=?1", params![project])?;
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

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<SessionRecord> {
    Ok(SessionRecord {
        session_id: row.get(0)?,
        agent_slug: row.get(1)?,
        agent_pubkey: row.get(2)?,
        project: row.get(3)?,
        host: row.get(4)?,
        child_pid: row.get(5)?,
        watch_pid: row.get(6)?,
        created_at: row.get(7)?,
        alive: row.get::<_, i32>(8)? != 0,
        rel_cwd: row.get(9)?,
    })
}

/// Column order: session_id, pubkey, slug, project, host, last_seen, rel_cwd.
fn row_to_peer(row: &rusqlite::Row) -> rusqlite::Result<PeerSession> {
    Ok(PeerSession {
        session_id: row.get(0)?,
        pubkey: row.get(1)?,
        slug: row.get(2)?,
        project: row.get(3)?,
        host: row.get(4)?,
        last_seen: row.get(5)?,
        rel_cwd: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session(id: &str) -> SessionRecord {
        SessionRecord {
            session_id: id.into(),
            agent_slug: "coder".into(),
            agent_pubkey: "pk-coder".into(),
            project: "proj".into(),
            host: "laptop".into(),
            child_pid: Some(42),
            watch_pid: Some(7),
            created_at: 1000,
            alive: true,
            rel_cwd: String::new(),
        }
    }

    #[test]
    fn session_roundtrip_and_death() {
        let s = Store::open_memory().unwrap();
        s.upsert_session(&sample_session("sess-1")).unwrap();
        assert_eq!(
            s.get_session("sess-1").unwrap().unwrap(),
            sample_session("sess-1")
        );
        assert_eq!(s.list_alive_sessions().unwrap().len(), 1);
        s.mark_session_dead("sess-1").unwrap();
        assert!(s.list_alive_sessions().unwrap().is_empty());
        assert!(!s.get_session("sess-1").unwrap().unwrap().alive);
    }

    #[test]
    fn inbox_is_idempotent_per_session() {
        let s = Store::open_memory().unwrap();
        let row = InboxRow {
            mention_event_id: "evt-1".into(),
            target_session: "sess-A".into(),
            from_pubkey: "pk".into(),
            from_slug: "reviewer".into(),
            project: "proj".into(),
            body: "look here".into(),
            created_at: 5,
            from_session: "sender-A".into(),
        };
        assert!(s.enqueue_mention(&row).unwrap()); // new
        assert!(!s.enqueue_mention(&row).unwrap()); // duplicate ignored
                                                    // same mention, different session = distinct delivery
        let mut other = row.clone();
        other.target_session = "sess-B".into();
        assert!(s.enqueue_mention(&other).unwrap());

        let drained = s.drain_inbox("sess-A").unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].body, "look here");
        assert!(s.drain_inbox("sess-A").unwrap().is_empty()); // delivered once
        assert_eq!(s.drain_inbox("sess-B").unwrap().len(), 1);
    }

    /// Bug C (agent-scoped sender resolution): the latest-alive fallback must be
    /// scoped to the invoking agent, not the most-recently-active session of ANY
    /// agent in the project. Otherwise a `claude` send is recorded as `opencode`
    /// merely because opencode was the latest-active session.
    #[test]
    fn latest_alive_session_is_agent_scoped() {
        let s = Store::open_memory().unwrap();
        let mut claude = sample_session("sess-claude");
        claude.agent_slug = "claude".into();
        claude.agent_pubkey = "pk-claude".into();
        claude.created_at = 100;
        s.upsert_session(&claude).unwrap();

        let mut opencode = sample_session("sess-opencode");
        opencode.agent_slug = "opencode".into();
        opencode.agent_pubkey = "pk-opencode".into();
        opencode.created_at = 200; // more recently active
        s.upsert_session(&opencode).unwrap();

        // Agent-agnostic lookup returns opencode (the latest active) — the BUG.
        assert_eq!(
            s.latest_alive_session_for_project("proj").unwrap().unwrap().agent_slug,
            "opencode"
        );
        // Agent-scoped lookup honors the invoking agent.
        assert_eq!(
            s.latest_alive_session_for_agent_in_project("claude", "proj")
                .unwrap()
                .unwrap()
                .agent_slug,
            "claude"
        );
        assert_eq!(
            s.latest_alive_session_for_agent_in_project("opencode", "proj")
                .unwrap()
                .unwrap()
                .agent_slug,
            "opencode"
        );
        // No alive session for an unknown agent.
        assert!(s
            .latest_alive_session_for_agent_in_project("codex", "proj")
            .unwrap()
            .is_none());
    }

    #[test]
    fn resolve_with_project_scope_prefers_matching_presence() {
        let s = Store::open_memory().unwrap();
        s.upsert_peer_session("sess-x", "pk-from-presence", "reviewer", "proj", "host", "", 1)
            .unwrap();
        assert_eq!(
            s.resolve_agent_pubkey("reviewer", Some("proj"))
                .unwrap()
                .as_deref(),
            Some("pk-from-presence")
        );
        s.upsert_profile("pk-from-profile", "reviewer", "host", 2)
            .unwrap();
        assert_eq!(
            s.resolve_agent_pubkey("reviewer", Some("proj"))
                .unwrap()
                .as_deref(),
            Some("pk-from-presence")
        );
        assert_eq!(
            s.resolve_agent_pubkey("reviewer", Some("other"))
                .unwrap()
                .as_deref(),
            None
        );
        assert_eq!(
            s.resolve_agent_pubkey("reviewer", None)
                .unwrap()
                .as_deref(),
            Some("pk-from-profile")
        );
    }

    #[test]
    fn peer_freshness_and_prune() {
        let s = Store::open_memory().unwrap();
        s.upsert_peer_session("old", "pk1", "stale", "proj", "h", "", 100)
            .unwrap();
        s.upsert_peer_session("new", "pk2", "live", "proj", "h", "", 1000)
            .unwrap();
        // since=500 → only the fresh one is "live"
        let live = s.list_peer_sessions(Some("proj"), 500).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].slug, "live");
        // since=0 → both
        assert_eq!(s.list_peer_sessions(Some("proj"), 0).unwrap().len(), 2);
        // prune removes the stale one
        assert_eq!(s.prune_peer_sessions(500).unwrap(), 1);
        assert_eq!(s.list_peer_sessions(Some("proj"), 0).unwrap().len(), 1);
    }

    #[test]
    fn rel_cwd_persists_on_peer_and_own_sessions() {
        let s = Store::open_memory().unwrap();
        // Peer session learns rel_cwd from presence.
        s.upsert_peer_session("p1", "pk", "rev", "proj", "tower", "worktree2", 1_000)
            .unwrap();
        let peers = s.list_peer_sessions(Some("proj"), 0).unwrap();
        assert_eq!(peers[0].rel_cwd, "worktree2");
        // Updating keeps the latest rel_cwd.
        s.upsert_peer_session("p1", "pk", "rev", "proj", "tower", "sub/dir", 1_001)
            .unwrap();
        assert_eq!(s.list_peer_sessions(Some("proj"), 0).unwrap()[0].rel_cwd, "sub/dir");

        // Own session stores + reads back rel_cwd (needed by reconcile).
        s.upsert_session(&sample_session("mine")).unwrap();
        let mut rec = sample_session("mine");
        rec.rel_cwd = "worktree1".into();
        s.upsert_session(&rec).unwrap();
        assert_eq!(s.get_session("mine").unwrap().unwrap().rel_cwd, "worktree1");
    }

    #[test]
    fn rel_cwd_migration_is_idempotent_on_reopen() {
        // Opening an on-disk db twice must not fail on the guarded ALTER TABLE
        // (the column already exists the second time).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let s = Store::open(&path).unwrap();
            let mut rec = sample_session("m");
            rec.rel_cwd = "wt".into();
            s.upsert_session(&rec).unwrap();
        }
        let s2 = Store::open(&path).unwrap();
        assert_eq!(s2.get_session("m").unwrap().unwrap().rel_cwd, "wt");
    }

    #[test]
    fn pending_agents_lifecycle() {
        let s = Store::open_memory().unwrap();
        s.upsert_pending_agent("pkX", "intruder", "their-box", "owner1", 5)
            .unwrap();
        s.upsert_pending_agent("pkX", "intruder", "their-box", "owner1", 6)
            .unwrap(); // upsert
        let pend = s.list_pending_agents().unwrap();
        assert_eq!(pend.len(), 1);
        assert_eq!(pend[0].slug, "intruder");
        s.remove_pending_agent("pkX").unwrap();
        assert!(s.list_pending_agents().unwrap().is_empty());
    }

    #[test]
    fn session_prefix_lookup() {
        let s = Store::open_memory().unwrap();
        s.upsert_peer_session("abcdef123456", "pk", "coder", "proj", "host", "", 1)
            .unwrap();
        let found = s.find_peer_session_by_prefix("abcdef").unwrap().unwrap();
        assert_eq!(found.pubkey, "pk");
        assert!(s.find_peer_session_by_prefix("zzzz").unwrap().is_none());
    }

    #[test]
    fn turn_delta_peer_sessions_can_be_project_scoped() {
        let s = Store::open_memory().unwrap();
        s.upsert_peer_session("sess-a", "pk-a", "same", "current", "host", "", 100)
            .unwrap();
        s.upsert_peer_session("sess-b", "pk-b", "other", "elsewhere", "host", "", 100)
            .unwrap();

        let scoped = s.list_new_peer_sessions(50, 50, Some("current")).unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].session_id, "sess-a");

        let all = s.list_new_peer_sessions(50, 50, None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn turn_delta_status_changes_can_be_project_scoped() {
        let s = Store::open_memory().unwrap();
        s.upsert_profile("pk-a", "alpha", "host", 1).unwrap();
        s.upsert_profile("pk-b", "bravo", "host", 1).unwrap();
        s.set_agent_status("pk-a", "current", "working here", 100)
            .unwrap();
        s.set_agent_status("pk-b", "elsewhere", "working there", 100)
            .unwrap();

        let scoped = s.list_status_changes_since(50, Some("current")).unwrap();
        assert_eq!(
            scoped,
            vec![(
                "alpha".to_string(),
                "current".to_string(),
                "working here".to_string()
            )]
        );

        let all = s.list_status_changes_since(50, None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn owned_groups_roundtrip_and_idempotent() {
        let s = Store::open_memory().unwrap();
        assert!(!s.is_group_owned("proj").unwrap());
        s.mark_group_owned("proj", 100).unwrap();
        assert!(s.is_group_owned("proj").unwrap());
        // Re-marking is a no-op (keeps the original created_at), not an error.
        s.mark_group_owned("proj", 200).unwrap();
        assert!(s.is_group_owned("proj").unwrap());
        assert!(!s.is_group_owned("other").unwrap());
    }

    #[test]
    fn group_member_upsert_and_query() {
        let s = Store::open_memory().unwrap();
        assert!(!s.is_group_member("proj", "pk-a").unwrap());
        s.upsert_group_member("proj", "pk-a", "member", 100).unwrap();
        assert!(s.is_group_member("proj", "pk-a").unwrap());
        // Membership is per (project, pubkey).
        assert!(!s.is_group_member("other", "pk-a").unwrap());
        assert!(!s.is_group_member("proj", "pk-b").unwrap());
        // Upsert is idempotent on the primary key.
        s.upsert_group_member("proj", "pk-a", "admin", 200).unwrap();
        assert!(s.is_group_member("proj", "pk-a").unwrap());
    }

    #[test]
    fn replace_group_members_is_authoritative() {
        let s = Store::open_memory().unwrap();
        s.upsert_group_member("proj", "stale", "member", 100).unwrap();
        // A relay 39002 snapshot replaces the whole set: 'stale' drops out.
        s.replace_group_members(
            "proj",
            &[("pk-a".into(), "member".into()), ("pk-b".into(), "admin".into())],
            300,
        )
        .unwrap();
        assert!(!s.is_group_member("proj", "stale").unwrap());
        assert!(s.is_group_member("proj", "pk-a").unwrap());
        assert!(s.is_group_member("proj", "pk-b").unwrap());
        // Scoped to the project — a different group is untouched.
        s.upsert_group_member("other", "pk-x", "member", 100).unwrap();
        s.replace_group_members("proj", &[], 400).unwrap();
        assert!(!s.is_group_member("proj", "pk-a").unwrap());
        assert!(s.is_group_member("other", "pk-x").unwrap());
    }
}
