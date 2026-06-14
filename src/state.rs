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
use std::sync::atomic::{AtomicU64, Ordering};

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
pub struct InboxRow {
    pub mention_event_id: String,
    pub target_session: String,
    pub from_pubkey: String,
    pub from_slug: String,
    pub project: String,
    pub body: String,
    /// When the sender published this mention (the kind:1 event timestamp), so the
    /// envelope's Date reflects send time, not local receipt/route time.
    pub created_at: u64,
    /// The sender's session id (empty when unknown — old peers / untargeted).
    /// Lets the recipient reply to the exact sibling session that wrote this.
    pub from_session: String,
    /// Envelope: one-line subject ("" when unset).
    pub subject: String,
    /// Envelope: sender's git branch at send time ("" outside a repo).
    pub branch: String,
    /// Envelope: sender's short commit hash at send time ("" outside a repo).
    pub commit: String,
    /// Envelope: count of dirty, non-gitignored files in the sender's tree.
    pub dirty: u32,
    /// Envelope: sender's host label (drives the `[remote: <host>]` annotation).
    pub host: String,
}

// ── Phase 7 read-model types ─────────────────────────────────────────────────

/// Enriched thread summary returned by `list_threads` and `thread_meta`.
///
/// `last_message_at` is `None` when the thread has no messages yet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThreadMeta {
    pub thread_id: String,
    pub project_id: String,
    pub subject: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: u64,
    pub last_message_at: Option<u64>,
}

/// One canonical message row returned by `messages_for_thread`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MessageRow {
    pub message_id: String,
    pub thread_id: String,
    pub author_pubkey: String,
    pub body: String,
    pub created_at: u64,
    pub direction: String,
    pub sync_state: String,
    pub native_event_id: Option<String>,
}

// ── Phase 1 read-model types ─────────────────────────────────────────────────

/// Whether a pubkey is a member of a project at a given timestamp.
///
/// - `Unhydrated` — no membership rows exist at all for `project_id`; the
///   admission path must quarantine inbound events until a backfill arrives.
/// - `NotMember` — rows exist for the project but not this pubkey.
/// - `Member` — an admitted, not-yet-revoked row.
/// - `Revoked` — a row whose `revoked_at <= ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembershipDecision {
    Member { role: String },
    Revoked,
    NotMember,
    Unhydrated,
}

/// One row from the `inbound_quarantine` table, returned by `replay_quarantine`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedEnvelope {
    pub native_event_id: String,
    pub project_id: Option<String>,
    pub reason: String,
    pub raw_envelope: String,
    pub created_at: u64,
}

// ── ID generation ────────────────────────────────────────────────────────────
// No uuid crate in Cargo.toml, so we build collision-resistant ids from
// nanosecond wall-clock time + an in-process monotonic counter.  Format:
//   te-<nanos_hex>-<counter_hex>
// The counter prevents collisions inside tight backfill loops where two calls
// may land within the same nanosecond.

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn gen_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{nanos:x}-{seq:x}")
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
    delivered_at     INTEGER NOT NULL DEFAULT 0,
    from_session     TEXT NOT NULL DEFAULT '',
    subject          TEXT NOT NULL DEFAULT '',
    branch           TEXT NOT NULL DEFAULT '',
    commit_hash      TEXT NOT NULL DEFAULT '',
    dirty            INTEGER NOT NULL DEFAULT 0,
    host             TEXT NOT NULL DEFAULT '',
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
    activity   TEXT NOT NULL DEFAULT '',
    active     INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (pubkey, project)
);
-- Current status scoped to one concrete session. This avoids showing one
-- Claude/Codex turn beside every sibling session that shares the same agent
-- pubkey in a project. `agent_status` remains as a legacy fallback for older
-- peers that publish agent-level status without a session-id tag.
CREATE TABLE IF NOT EXISTS session_status (
    pubkey     TEXT NOT NULL,
    project    TEXT NOT NULL,
    session_id TEXT NOT NULL,
    text       TEXT NOT NULL,
    activity   TEXT NOT NULL DEFAULT '',
    active     INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (pubkey, project, session_id)
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

-- ── Phase 1: canonical read-model tables ──────────────────────────────────────
-- Durable project identities with surrogate ids; origin tables map fabric
-- coordinates back to local ids.
CREATE TABLE IF NOT EXISTS projects (
    project_id   TEXT PRIMARY KEY,
    display_slug TEXT NOT NULL,
    about        TEXT,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS project_origins (
    project_id           TEXT NOT NULL,
    fabric               TEXT NOT NULL,
    provider_instance    TEXT NOT NULL,
    native_project_key   TEXT NOT NULL,
    UNIQUE(fabric, provider_instance, native_project_key)
);
CREATE TABLE IF NOT EXISTS threads (
    thread_id   TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    subject     TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    archived_at INTEGER
);
CREATE TABLE IF NOT EXISTS thread_origins (
    thread_id            TEXT NOT NULL,
    fabric               TEXT NOT NULL,
    provider_instance    TEXT NOT NULL,
    native_thread_key    TEXT NOT NULL,
    UNIQUE(fabric, provider_instance, native_thread_key)
);
-- author_session is the return envelope (the sender's session id so a reply can
-- target the exact sibling session that wrote the message; NULL when the fabric
-- can't supply it — reply degrades to agent-level). Populated during dual-write
-- in a later phase; schema included now per the doc (§2a + Phase 1 spec).
CREATE TABLE IF NOT EXISTS messages (
    message_id      TEXT PRIMARY KEY,
    thread_id       TEXT NOT NULL,
    author_pubkey   TEXT NOT NULL,
    author_session  TEXT,
    body            TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    direction       TEXT NOT NULL,
    sync_state      TEXT NOT NULL,
    native_event_id TEXT,
    error           TEXT
);
-- message_recipients PK includes target_session which can be NULL.  SQLite
-- treats NULL values as distinct in a UNIQUE / PRIMARY KEY constraint, so two
-- rows with the same (message_id, recipient_pubkey) but NULL target_session are
-- considered different rows.  That behaviour is acceptable here.
CREATE TABLE IF NOT EXISTS message_recipients (
    message_id       TEXT NOT NULL,
    recipient_pubkey TEXT NOT NULL,
    target_session   TEXT,
    delivered_at     INTEGER,
    PRIMARY KEY(message_id, recipient_pubkey, target_session)
);
CREATE TABLE IF NOT EXISTS inbound_quarantine (
    native_event_id TEXT PRIMARY KEY,
    project_id      TEXT,
    reason          TEXT NOT NULL,
    raw_envelope    TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS membership (
    project_id  TEXT NOT NULL,
    pubkey      TEXT NOT NULL,
    role        TEXT NOT NULL,
    admitted_at INTEGER NOT NULL,
    revoked_at  INTEGER,
    source      TEXT NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY(project_id, pubkey)
);
-- TMUX control-plane: one row per (session, kind='tmux') endpoint. Written by
-- rpc_session_start when the hook env supplies TMUX_PANE; read by the doorbell
-- dispatcher. `target` is the stable tmux pane id (e.g. '%5'). `meta` is a JSON
-- object that may carry {"socket":"...", "pane_command":"claude"}.
CREATE TABLE IF NOT EXISTS session_endpoints (
    session_id    TEXT NOT NULL,
    kind          TEXT NOT NULL,
    target        TEXT NOT NULL,
    meta          TEXT NOT NULL DEFAULT '',
    registered_at INTEGER NOT NULL,
    last_verified INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, kind)
);
-- Absolute project path indexed by project slug. Populated by session_start so
-- the tmux spawn command knows where to cd.
CREATE TABLE IF NOT EXISTS project_paths (
    project    TEXT PRIMARY KEY,
    abs_path   TEXT NOT NULL,
    updated_at INTEGER NOT NULL
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
        // Envelope columns: subject + the sender's workspace snapshot at send time.
        for col in [
            "subject TEXT NOT NULL DEFAULT ''",
            "branch TEXT NOT NULL DEFAULT ''",
            "commit_hash TEXT NOT NULL DEFAULT ''",
            "dirty INTEGER NOT NULL DEFAULT 0",
            "host TEXT NOT NULL DEFAULT ''",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE inbox ADD COLUMN {col}"), []);
        }
        // When a mention was drained to its session — drives the statusline's
        // "recently consumed" inbox segment.
        let _ = conn.execute(
            "ALTER TABLE inbox ADD COLUMN delivered_at INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // NIP-10 thread tracking: root event (first user prompt in the session
        // thread) and most recent user prompt (triggers TurnReply at stop-hook).
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN thread_root_event_id TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN last_prompt_event_id TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Track the most recent TurnReply event ID so the next user prompt can
        // carry the correct NIP-10 reply marker threading back to the agent reply.
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN last_agent_reply_event_id TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Snapshot of the last assistant text at the beginning of each turn.
        // Used by rpc_turn_end to poll until a *new* response appears in the
        // transcript (Claude Code writes the transcript after the stop hook fires).
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN last_assistant_text_at_turn_start TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Persistent session title now carries a separate mid-turn flag; the title
        // (text) survives idle turns and `active` drives the idle marker.
        let _ = conn.execute(
            "ALTER TABLE session_status ADD COLUMN active INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE agent_status ADD COLUMN active INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Live "what it's doing now" line, distilled alongside the title in one
        // call. Separate from `text` (the persistent title) so it can be cleared
        // on idle while the title survives.
        let _ = conn.execute(
            "ALTER TABLE session_status ADD COLUMN activity TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE agent_status ADD COLUMN activity TEXT NOT NULL DEFAULT ''",
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
        let rows = stmt.query_map([], row_to_session)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn list_local_agent_pubkeys(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT agent_pubkey FROM sessions")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
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
    pub fn set_thread_event_ids(
        &self,
        session_id: &str,
        root_id: &str,
        prompt_id: &str,
    ) -> Result<()> {
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
    pub fn set_last_assistant_text_at_turn_start(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<()> {
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
        let rows = stmt.query_map(params![since], row_to_session)?;
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

        if let Ok(pk) = self.conn.query_row(
            "SELECT pubkey FROM profiles WHERE slug=?1 ORDER BY updated_at DESC LIMIT 1",
            params![slug],
            |r| r.get::<_, String>(0),
        ) {
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

    /// Reverse-lookup: given a pubkey, return the slug this agent is known by
    /// (from own sessions, peer_sessions, or profiles). Returns None if completely unknown.
    /// Look up the agent slug for a locally-owned pubkey from the `sessions`
    /// table (including `alive=0` rows). Returns `None` for remote-only pubkeys
    /// that have no local session record — callers use this as the "is locally
    /// owned?" gate before attempting a tmux spawn.
    pub fn get_local_agent_slug_by_pubkey(&self, pubkey: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT agent_slug FROM sessions WHERE agent_pubkey=?1 ORDER BY created_at DESC LIMIT 1",
                params![pubkey],
                |r| r.get::<_, String>(0),
            )
            .ok()
    }

    pub fn resolve_slug_for_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        // Check own sessions first (most authoritative for local agents).
        if let Ok(slug) = self.conn.query_row(
            "SELECT agent_slug FROM sessions WHERE agent_pubkey=?1 ORDER BY created_at DESC LIMIT 1",
            params![pubkey],
            |r| r.get::<_, String>(0),
        ) {
            return Ok(Some(slug));
        }
        // Then peer_sessions (remote agents seen recently).
        if let Ok(slug) = self.conn.query_row(
            "SELECT slug FROM peer_sessions WHERE pubkey=?1 ORDER BY last_seen DESC LIMIT 1",
            params![pubkey],
            |r| r.get::<_, String>(0),
        ) {
            return Ok(Some(slug));
        }
        // Fall back to profiles table.
        Ok(self
            .conn
            .query_row(
                "SELECT slug FROM profiles WHERE pubkey=?1 LIMIT 1",
                params![pubkey],
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

    // ── inbox ────────────────────────────────────────────────────────────

    /// Idempotent insert. Returns true if the row was newly stored.
    pub fn enqueue_mention(&self, m: &InboxRow) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO inbox
               (mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, delivered, from_session, subject, branch, commit_hash, dirty, host)
             VALUES (?1,?2,?3,?4,?5,?6,?7,0,?8,?9,?10,?11,?12,?13)",
            params![
                m.mention_event_id, m.target_session, m.from_pubkey, m.from_slug,
                m.project, m.body, m.created_at, m.from_session,
                m.subject, m.branch, m.commit, m.dirty, m.host
            ],
        )?;
        Ok(changed > 0)
    }

    /// Read undelivered mentions without marking them delivered. Safe for
    /// mid-turn checks (turn_check) — no writes to state.db.
    pub fn peek_inbox(&self, session_id: &str) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session, subject, branch, commit_hash, dirty, host
             FROM inbox WHERE target_session=?1 AND delivered=0 ORDER BY created_at",
        )?;
        let rows: Vec<InboxRow> = stmt
            .query_map(params![session_id], row_to_inbox)?
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

    /// Agent status rows updated at or after `since`. Returns (slug, project, title, session_id, active).
    /// `session_id` is `None` for legacy `agent_status` rows and `Some(id)` for `session_status` rows.
    /// `active` is the mid-turn flag (idle = !active); the title persists across idle turns.
    /// Resolves slug from profiles then peer_sessions, falling back to "unknown".
    pub fn list_status_changes_since(
        &self,
        since: u64,
        project: Option<&str>,
    ) -> Result<Vec<(String, String, String, Option<String>, bool)>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(
                 (SELECT slug FROM profiles WHERE pubkey=ast.pubkey LIMIT 1),
                 (SELECT slug FROM peer_sessions WHERE pubkey=ast.pubkey ORDER BY last_seen DESC LIMIT 1),
                 'unknown'
             ), ast.project, ast.text, NULL, ast.active, ast.updated_at
             FROM agent_status ast
             WHERE ast.updated_at>=?1 AND (?2 IS NULL OR ast.project=?2)
             UNION ALL
             SELECT COALESCE(
                 (SELECT agent_slug FROM sessions WHERE session_id=sst.session_id LIMIT 1),
                 (SELECT slug FROM peer_sessions WHERE session_id=sst.session_id LIMIT 1),
                 (SELECT slug FROM profiles WHERE pubkey=sst.pubkey LIMIT 1),
                 'unknown'
             ), sst.project, sst.text, sst.session_id, sst.active, sst.updated_at
             FROM session_status sst
             WHERE sst.updated_at>=?1 AND (?2 IS NULL OR sst.project=?2)
             ORDER BY 6",
        )?;
        let rows: Vec<(String, String, String, Option<String>, bool)> = stmt
            .query_map(params![since, project], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get::<_, i64>(4)? != 0,
                ))
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

    /// Returns `true` if the session is currently mid-turn (`working = 1`).
    /// Defaults to `false` (not working) when no row exists — fail-open so
    /// the doorbell is allowed when a session has never started a turn.
    pub fn is_session_working(&self, session_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT working FROM turn_state WHERE session_id=?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            != 0
    }

    /// Count undelivered mentions for a session without consuming them.
    pub fn count_unread_inbox(&self, session_id: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM inbox WHERE target_session=?1 AND delivered=0",
            params![session_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
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

    #[allow(clippy::too_many_arguments)]
    pub fn set_agent_status(
        &self,
        pubkey: &str,
        project: &str,
        session_id: Option<&str>,
        text: &str,
        activity: &str,
        active: bool,
        ts: u64,
    ) -> Result<()> {
        let active = active as i64;
        if let Some(session_id) = session_id.filter(|s| !s.is_empty()) {
            self.conn.execute(
                "INSERT INTO session_status (pubkey, project, session_id, text, activity, active, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(pubkey, project, session_id) DO UPDATE SET text=?4, activity=?5, active=?6, updated_at=?7",
                params![pubkey, project, session_id, text, activity, active, ts],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO agent_status (pubkey, project, text, activity, active, updated_at) VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(pubkey, project) DO UPDATE SET text=?3, activity=?4, active=?5, updated_at=?6",
                params![pubkey, project, text, activity, active, ts],
            )?;
        }
        Ok(())
    }

    /// Current (title, activity, active) for an agent/session. Session-scoped row
    /// first, agent-level fallback. The title persists across idle turns;
    /// `activity` is the live "doing now" line (empty when idle/unknown);
    /// `active` is the live mid-turn flag.
    pub fn get_agent_status(
        &self,
        pubkey: &str,
        project: &str,
        session_id: Option<&str>,
    ) -> Result<Option<(String, String, bool)>> {
        if let Some(session_id) = session_id.filter(|s| !s.is_empty()) {
            if let Ok(row) = self.conn.query_row(
                "SELECT text, activity, active FROM session_status
                     WHERE pubkey=?1 AND project=?2 AND session_id=?3",
                params![pubkey, project, session_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)? != 0,
                    ))
                },
            ) {
                return Ok(Some(row));
            }
        }
        Ok(self
            .conn
            .query_row(
                "SELECT text, activity, active FROM agent_status WHERE pubkey=?1 AND project=?2",
                params![pubkey, project],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)? != 0,
                    ))
                },
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

    /// Cached NIP-29 roster size for a project (0 when membership is unknown,
    /// e.g. no userNsec → no group management → empty cache).
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

    /// Return undelivered mentions for a session and mark them delivered.
    pub fn drain_inbox(&self, session_id: &str) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session, subject, branch, commit_hash, dirty, host
             FROM inbox WHERE target_session=?1 AND delivered=0 ORDER BY created_at",
        )?;
        let rows: Vec<InboxRow> = stmt
            .query_map(params![session_id], row_to_inbox)?
            .filter_map(|r| r.ok())
            .collect();
        self.conn.execute(
            "UPDATE inbox SET delivered=1, delivered_at=?2 WHERE target_session=?1 AND delivered=0",
            params![session_id, crate::util::now_secs()],
        )?;
        Ok(rows)
    }

    // ── Phase 1: canonical read-model accessors ──────────────────────────
    // Write-side primitives the materializer fills; readers come in Phase 2.
    // These tables are additive — no existing reader consults them yet, so
    // none of this changes CLI/RPC output.

    /// Map fabric coordinates to a durable `project_id`, creating the project +
    /// origin on first sight. Idempotent: the same origin always resolves to the
    /// same id and never clobbers `about`.
    pub fn ensure_project_origin(
        &self,
        fabric: &str,
        provider_instance: &str,
        native_project_key: &str,
        display_slug: &str,
        now: u64,
    ) -> Result<String> {
        if let Some(pid) =
            self.project_id_for_origin(fabric, provider_instance, native_project_key)?
        {
            return Ok(pid);
        }
        let pid = gen_id("proj");
        self.conn.execute(
            "INSERT INTO projects (project_id, display_slug, about, created_at, updated_at)
             VALUES (?1, ?2, NULL, ?3, ?3)",
            params![pid, display_slug, now],
        )?;
        self.conn.execute(
            "INSERT INTO project_origins (project_id, fabric, provider_instance, native_project_key)
             VALUES (?1, ?2, ?3, ?4)",
            params![pid, fabric, provider_instance, native_project_key],
        )?;
        Ok(pid)
    }

    pub fn project_id_for_origin(
        &self,
        fabric: &str,
        provider_instance: &str,
        native_project_key: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT project_id FROM project_origins
                 WHERE fabric=?1 AND provider_instance=?2 AND native_project_key=?3",
                params![fabric, provider_instance, native_project_key],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    /// Map a fabric thread key to a durable `thread_id`, creating the thread +
    /// origin on first sight. Idempotent.
    pub fn ensure_thread_origin(
        &self,
        project_id: &str,
        fabric: &str,
        provider_instance: &str,
        native_thread_key: &str,
        now: u64,
    ) -> Result<String> {
        if let Ok(tid) = self.conn.query_row(
            "SELECT thread_id FROM thread_origins
                 WHERE fabric=?1 AND provider_instance=?2 AND native_thread_key=?3",
            params![fabric, provider_instance, native_thread_key],
            |r| r.get::<_, String>(0),
        ) {
            return Ok(tid);
        }
        let tid = gen_id("thr");
        self.conn.execute(
            "INSERT INTO threads (thread_id, project_id, subject, created_at, updated_at)
             VALUES (?1, ?2, NULL, ?3, ?3)",
            params![tid, project_id, now],
        )?;
        self.conn.execute(
            "INSERT INTO thread_origins (thread_id, fabric, provider_instance, native_thread_key)
             VALUES (?1, ?2, ?3, ?4)",
            params![tid, fabric, provider_instance, native_thread_key],
        )?;
        Ok(tid)
    }

    /// Insert a canonical message, returning its `message_id`. When
    /// `native_event_id` is `Some` this is idempotent: a message already carrying
    /// that native id is returned rather than duplicated (relay echo / refetch).
    #[allow(clippy::too_many_arguments)] // one param per messages column; a struct would only move the noise
    pub fn record_message(
        &self,
        thread_id: &str,
        author_pubkey: &str,
        body: &str,
        created_at: u64,
        direction: &str,
        sync_state: &str,
        native_event_id: Option<&str>,
    ) -> Result<String> {
        if let Some(eid) = native_event_id {
            if let Ok(mid) = self.conn.query_row(
                "SELECT message_id FROM messages WHERE native_event_id=?1",
                params![eid],
                |r| r.get::<_, String>(0),
            ) {
                return Ok(mid);
            }
        }
        let mid = gen_id("msg");
        self.conn.execute(
            "INSERT INTO messages
               (message_id, thread_id, author_pubkey, body, created_at, direction, sync_state, native_event_id, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
            params![mid, thread_id, author_pubkey, body, created_at, direction, sync_state, native_event_id],
        )?;
        Ok(mid)
    }

    /// Canonical thread that contains the message published as `native_event_id`.
    /// Used by `inbox reply` to file the reply into the original's thread.
    pub fn thread_for_native_event(&self, native_event_id: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT thread_id FROM messages WHERE native_event_id=?1",
                params![native_event_id],
                |r| r.get::<_, String>(0),
            )
            .ok()
    }

    pub fn mark_message_sync_state(
        &self,
        message_id: &str,
        sync_state: &str,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET sync_state=?2, error=?3 WHERE message_id=?1",
            params![message_id, sync_state, error],
        )?;
        Ok(())
    }

    /// Idempotent. `target_session = None` stores a NULL addressee (SQLite treats
    /// NULL as distinct in the PK, matching the doc's recipient model).
    pub fn add_message_recipient(
        &self,
        message_id: &str,
        recipient_pubkey: &str,
        target_session: Option<&str>,
    ) -> Result<()> {
        match target_session {
            Some(ts) => {
                self.conn.execute(
                    "INSERT OR IGNORE INTO message_recipients
                       (message_id, recipient_pubkey, target_session, delivered_at)
                     VALUES (?1, ?2, ?3, NULL)",
                    params![message_id, recipient_pubkey, ts],
                )?;
            }
            None => {
                // SQLite treats NULL as DISTINCT in the PK, so INSERT OR IGNORE
                // does NOT dedup an untargeted (NULL target_session) recipient —
                // repeated materialization (relay echo + catch-up refetch) would
                // otherwise accumulate one duplicate row per re-delivery. Guard
                // with an explicit existence check so it stays idempotent.
                self.conn.execute(
                    "INSERT INTO message_recipients
                       (message_id, recipient_pubkey, target_session, delivered_at)
                     SELECT ?1, ?2, NULL, NULL
                     WHERE NOT EXISTS (
                       SELECT 1 FROM message_recipients
                       WHERE message_id=?1 AND recipient_pubkey=?2 AND target_session IS NULL
                     )",
                    params![message_id, recipient_pubkey],
                )?;
            }
        }
        Ok(())
    }

    /// Admit (or re-admit) a member. Upsert: preserves the original `admitted_at`,
    /// clears any prior `revoked_at`, refreshes role/source/updated_at.
    pub fn admit_member(
        &self,
        project_id: &str,
        pubkey: &str,
        role: &str,
        source: &str,
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO membership (project_id, pubkey, role, admitted_at, revoked_at, source, updated_at)
             VALUES (?1, ?2, ?3, ?5, NULL, ?4, ?5)
             ON CONFLICT(project_id, pubkey) DO UPDATE SET
               role=excluded.role, source=excluded.source, revoked_at=NULL, updated_at=excluded.updated_at",
            params![project_id, pubkey, role, source, ts],
        )?;
        Ok(())
    }

    pub fn revoke_member(&self, project_id: &str, pubkey: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE membership SET revoked_at=?3, updated_at=?3 WHERE project_id=?1 AND pubkey=?2",
            params![project_id, pubkey, ts],
        )?;
        Ok(())
    }

    /// The admission predicate (write-side) and roster query (read-side) in one.
    /// `Unhydrated` (no rows at all for the project) is distinct from `NotMember`
    /// (rows exist, but not this pubkey) so the materializer can quarantine
    /// inbound events until membership arrives.
    pub fn is_member_at(
        &self,
        project_id: &str,
        pubkey: &str,
        ts: u64,
    ) -> Result<MembershipDecision> {
        let project_rows: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM membership WHERE project_id=?1",
            params![project_id],
            |r| r.get(0),
        )?;
        if project_rows == 0 {
            return Ok(MembershipDecision::Unhydrated);
        }
        let row: Option<(String, u64, Option<u64>)> = self
            .conn
            .query_row(
                "SELECT role, admitted_at, revoked_at FROM membership WHERE project_id=?1 AND pubkey=?2",
                params![project_id, pubkey],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        match row {
            None => Ok(MembershipDecision::NotMember),
            Some((role, admitted_at, revoked_at)) => {
                if let Some(rev) = revoked_at {
                    if rev <= ts {
                        return Ok(MembershipDecision::Revoked);
                    }
                }
                if admitted_at <= ts {
                    Ok(MembershipDecision::Member { role })
                } else {
                    // Admitted in the future relative to ts → not yet a member.
                    Ok(MembershipDecision::NotMember)
                }
            }
        }
    }

    /// Park an inbound event that could not be admitted yet. Idempotent on
    /// `native_event_id`.
    pub fn quarantine_inbound(
        &self,
        native_event_id: &str,
        project_id: Option<&str>,
        reason: &str,
        raw_envelope: &str,
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO inbound_quarantine
               (native_event_id, project_id, reason, raw_envelope, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![native_event_id, project_id, reason, raw_envelope, ts],
        )?;
        Ok(())
    }

    /// Quarantined envelopes awaiting replay, optionally filtered to one project.
    pub fn replay_quarantine(&self, project_id: Option<&str>) -> Result<Vec<QuarantinedEnvelope>> {
        let mut stmt = self.conn.prepare(
            "SELECT native_event_id, project_id, reason, raw_envelope, created_at
             FROM inbound_quarantine
             WHERE (?1 IS NULL OR project_id=?1)
             ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![project_id], |r| {
                Ok(QuarantinedEnvelope {
                    native_event_id: r.get(0)?,
                    project_id: r.get(1)?,
                    reason: r.get(2)?,
                    raw_envelope: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Drop a quarantined envelope once it has been replayed/admitted.
    pub fn clear_quarantine(&self, native_event_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM inbound_quarantine WHERE native_event_id=?1",
            params![native_event_id],
        )?;
        Ok(())
    }

    /// Backfill canonical project origins + membership from the legacy tables for
    /// the current kind1/nip29 fabric. `provider_instance` is the relay-set hash
    /// (the daemon derives it from config and passes it in — not this layer's job).
    /// Idempotent: re-running creates no duplicate origins or membership rows.
    pub fn backfill_kind1_nip29_origins(&self, provider_instance: &str, now: u64) -> Result<()> {
        const FABRIC: &str = "kind1-nip29";
        // Every project slug ever observed across the legacy tables.
        let slugs: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT project FROM project_meta
                 UNION SELECT project FROM sessions
                 UNION SELECT project FROM peer_sessions
                 UNION SELECT project FROM group_members",
            )?;
            let v: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            v
        };
        for slug in &slugs {
            let pid = self.ensure_project_origin(FABRIC, provider_instance, slug, slug, now)?;
            // project_meta is the authority for `about`; carry it onto the row.
            if let Some(about) = self.get_project_meta(slug)? {
                self.conn.execute(
                    "UPDATE projects SET about=?2, updated_at=?3 WHERE project_id=?1",
                    params![pid, about, now],
                )?;
            }
        }
        // Mirror the nip29 roster snapshot into canonical membership.
        let members: Vec<(String, String, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT project, pubkey, role FROM group_members")?;
            let v: Vec<(String, String, String)> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            v
        };
        for (project, pubkey, role) in &members {
            if let Some(pid) = self.project_id_for_origin(FABRIC, provider_instance, project)? {
                self.admit_member(&pid, pubkey, role, "nip29-39002", now)?;
            }
        }
        Ok(())
    }

    // ── Phase 2: read-model methods ──────────────────────────────────────────
    //
    // Every reader must go through one of these methods so that Phase 8 can
    // swap the underlying source without touching callers.  Methods that still
    // query legacy tables carry a TODO naming the removal phase.

    /// All known projects, ordered by slug.
    ///
    /// Currently backed by the legacy `project_meta` table (the only durable
    /// project list we have before dual-write is active).  Falls back to an
    /// empty vec when the table has no rows.
    ///
    // Retained storage (Phase 8): project_meta is the deliberately-retained canonical home for
    // project slug+about; readers query it directly per fabric-architecture.md §6.
    pub fn list_projects_read_model(&self) -> Result<Vec<(String, String)>> {
        self.list_project_meta()
    }

    /// About-text for a single project by its legacy slug.
    ///
    // Retained storage (Phase 8): project_meta is the deliberately-retained canonical home for
    // project slug+about; readers query it directly per fabric-architecture.md §6.
    pub fn project_meta_read_model(&self, slug: &str) -> Result<Option<String>> {
        self.get_project_meta(slug)
    }

    /// Own (local) sessions that are still alive and recently heartbeated.
    ///
    // Retained storage (Phase 8): sessions is the deliberately-retained canonical home for
    // local agent sessions; readers query it directly per fabric-architecture.md §6.
    pub fn list_agents_read_model(
        &self,
        project: Option<&str>,
        since: u64,
    ) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd
             FROM sessions WHERE alive=1 AND last_seen>=?1 AND (?2 IS NULL OR project=?2) ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![since, project], row_to_session)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Peer presence rows, ordered by recency.
    ///
    // Retained storage (Phase 8): peer_sessions is the deliberately-retained canonical home for
    // peer presence; readers query it directly per fabric-architecture.md §6.
    pub fn list_presence_read_model(
        &self,
        project: Option<&str>,
        since: u64,
    ) -> Result<Vec<PeerSession>> {
        self.list_peer_sessions(project, since)
    }

    /// Agent status for all agents in a project (or all projects when `project` is None).
    /// Returns `(pubkey, project, text)` tuples.
    ///
    // Retained storage (Phase 8): agent_status is the deliberately-retained canonical home for
    // agent status; readers query it directly per fabric-architecture.md §6.
    pub fn list_status_read_model(
        &self,
        project: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT pubkey, project, text FROM agent_status
             WHERE (?1 IS NULL OR project=?1) ORDER BY updated_at DESC",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![project], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Canonical threads for a project, ordered by last activity (most-active first).
    ///
    /// Ordering: `COALESCE(MAX(m.created_at), t.created_at) DESC` — threads with
    /// recent messages sort before inactive threads; threads with no messages at all
    /// sort by their own `created_at` DESC.
    ///
    /// `project_id` is the canonical surrogate key from `projects`.
    pub fn list_threads(&self, project_id: &str) -> Result<Vec<ThreadMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.thread_id, t.project_id, t.subject, t.created_at, t.updated_at,
                    COUNT(m.message_id) AS message_count,
                    MAX(m.created_at) AS last_message_at
             FROM threads t
             LEFT JOIN messages m ON m.thread_id = t.thread_id
             WHERE t.project_id = ?1
             GROUP BY t.thread_id
             ORDER BY COALESCE(MAX(m.created_at), t.created_at) DESC",
        )?;
        let rows: Vec<ThreadMeta> = stmt
            .query_map(params![project_id], |r| {
                Ok(ThreadMeta {
                    thread_id: r.get(0)?,
                    project_id: r.get(1)?,
                    subject: r.get(2)?,
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
                    message_count: r.get(5)?,
                    last_message_at: r.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Canonical messages for a thread, ordered by `created_at` ascending
    /// (chronological order for conversation rendering).
    pub fn messages_for_thread(&self, thread_id: &str) -> Result<Vec<MessageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT message_id, thread_id, author_pubkey, body, created_at,
                    direction, sync_state, native_event_id
             FROM messages WHERE thread_id=?1 ORDER BY created_at",
        )?;
        let rows: Vec<MessageRow> = stmt
            .query_map(params![thread_id], |r| {
                Ok(MessageRow {
                    message_id: r.get(0)?,
                    thread_id: r.get(1)?,
                    author_pubkey: r.get(2)?,
                    body: r.get(3)?,
                    created_at: r.get(4)?,
                    direction: r.get(5)?,
                    sync_state: r.get(6)?,
                    native_event_id: r.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Enriched metadata for a single thread by its canonical id.
    /// Returns `None` if no thread with that id exists.
    pub fn thread_meta(&self, thread_id: &str) -> Result<Option<ThreadMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.thread_id, t.project_id, t.subject, t.created_at, t.updated_at,
                    COUNT(m.message_id) AS message_count,
                    MAX(m.created_at) AS last_message_at
             FROM threads t
             LEFT JOIN messages m ON m.thread_id = t.thread_id
             WHERE t.thread_id = ?1
             GROUP BY t.thread_id",
        )?;
        let mut rows = stmt.query(params![thread_id])?;
        if let Some(r) = rows.next()? {
            Ok(Some(ThreadMeta {
                thread_id: r.get(0)?,
                project_id: r.get(1)?,
                subject: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
                message_count: r.get(5)?,
                last_message_at: r.get(6)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Query for tail backfill: returns recent messages with author/project/thread info.
    ///
    /// Each row: (created_at, body, author_pubkey, project_slug, thread_id, author_session)
    pub fn recent_messages_for_backfill(
        &self,
        project: Option<&str>,
        since: u64,
        limit: u64,
    ) -> Result<Vec<(u64, String, String, String, String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.created_at, m.body, m.author_pubkey, p.display_slug, m.thread_id, m.author_session
             FROM messages m
             JOIN threads t ON t.thread_id = m.thread_id
             JOIN projects p ON p.project_id = t.project_id
             WHERE (?1 IS NULL OR p.display_slug = ?1) AND m.created_at >= ?2
             ORDER BY m.created_at DESC LIMIT ?3",
        )?;
        let rows: Vec<(u64, String, String, String, String, Option<String>)> = stmt
            .query_map(params![project, since, limit], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Resolve the native relay key for a thread's root message.
    ///
    /// Used by `provider.send` when encoding a reply: the root `e` tag must
    /// carry the relay-native event id of the thread's originating message so
    /// the recipient's inbound materializer groups the reply into the same thread.
    ///
    /// Returns `None` when the thread has no registered origin (safe degradation:
    /// the caller publishes without the root tag, creating a new thread on the
    /// recipient's side).
    pub fn thread_root_native_key(
        &self,
        thread_id: &str,
        fabric: &str,
        provider_instance: &str,
    ) -> Option<String> {
        self.conn
            .query_row(
                "SELECT native_thread_key FROM thread_origins
                 WHERE thread_id=?1 AND fabric=?2 AND provider_instance=?3",
                params![thread_id, fabric, provider_instance],
                |r| r.get::<_, String>(0),
            )
            .ok()
    }

    /// Resolve a project display-slug to its canonical `project_id` for the
    /// kind1-nip29 fabric.  Read-only — does NOT create an origin.
    pub fn project_id_for_slug(
        &self,
        fabric: &str,
        provider_instance: &str,
        slug: &str,
    ) -> Result<Option<String>> {
        self.project_id_for_origin(fabric, provider_instance, slug)
    }

    /// Peek (non-destructive read) of undelivered inbox rows for a session.
    ///
    /// This is the read-model facade for turn_check and any other non-draining
    /// reader.  `assemble_turn_start_context` intentionally keeps its direct
    /// `drain_inbox` call because drain is a delivery write, not a read-model
    /// query; routing it through this method would change the peek/drain
    /// semantics that freeze tests pin.
    ///
    // Retained storage (Phase 8): inbox is the deliberately-retained canonical home for
    // per-session delivered/seen messages; readers query it directly per fabric-architecture.md §6.
    pub fn undelivered_messages_for_session(&self, session_id: &str) -> Result<Vec<InboxRow>> {
        self.peek_inbox(session_id)
    }

    /// Find any inbox row whose `mention_event_id` starts with `prefix` (the short
    /// `ID` shown in an envelope). Used by `inbox reply --id` to recover the
    /// original sender + event to thread the reply against. Returns the first
    /// match ordered by recency; `None` if nothing matches.
    /// Mentions already drained to `session_id` at or after `since`. Read-only —
    /// drives the statusline's "recently consumed" inbox segment.
    pub fn list_recently_delivered(&self, session_id: &str, since: u64) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session, subject, branch, commit_hash, dirty, host
             FROM inbox WHERE target_session=?1 AND delivered=1 AND delivered_at>=?2 AND from_pubkey<>'' ORDER BY created_at",
        )?;
        let rows: Vec<InboxRow> = stmt
            .query_map(params![session_id, since], row_to_inbox)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn find_inbox_by_event_prefix(&self, prefix: &str) -> Result<Option<InboxRow>> {
        let pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT mention_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session, subject, branch, commit_hash, dirty, host
             FROM inbox WHERE mention_event_id LIKE ?1 ORDER BY created_at DESC LIMIT 1",
        )?;
        let row = stmt
            .query_map(params![pattern], row_to_inbox)?
            .filter_map(|r| r.ok())
            .next();
        Ok(row)
    }

    // ── Phase 2: write-facing materializer methods ───────────────────────────
    //
    // These are the write surface the Phase 4 materializer will call.  Nothing
    // calls them in Phase 2; they exist so the seam compiles, so the signatures
    // are locked, and so unit tests prevent dead-code warnings.

    /// Upsert a peer profile (kind:0).  Wraps `upsert_profile`.
    pub fn materialize_profile(&self, pubkey: &str, slug: &str, host: &str, ts: u64) -> Result<()> {
        self.upsert_profile(pubkey, slug, host, ts)
    }

    /// Record / refresh a peer presence session (kind:0 + relay presence).
    /// Wraps `upsert_peer_session`.
    #[allow(clippy::too_many_arguments)] // mirrors upsert_peer_session's column set
    pub fn materialize_presence(
        &self,
        session_id: &str,
        pubkey: &str,
        slug: &str,
        project: &str,
        host: &str,
        rel_cwd: &str,
        ts: u64,
    ) -> Result<()> {
        self.upsert_peer_session(session_id, pubkey, slug, project, host, rel_cwd, ts)
    }

    /// Record the current NIP-38 status for an agent.  Wraps `set_agent_status`.
    #[allow(clippy::too_many_arguments)]
    pub fn materialize_status(
        &self,
        pubkey: &str,
        project: &str,
        session_id: Option<&str>,
        text: &str,
        activity: &str,
        active: bool,
        ts: u64,
    ) -> Result<()> {
        self.set_agent_status(pubkey, project, session_id, text, activity, active, ts)
    }

    /// Apply a relay-authoritative NIP-29 39002 membership snapshot:
    /// replaces the legacy `group_members` cache wholesale AND mirrors into
    /// canonical `membership` rows via `admit_member` (source `"nip29-39002"`).
    ///
    /// `provider_instance` is the relay-set hash (daemon-derived); used to
    /// resolve the canonical `project_id` via `project_id_for_origin`.
    pub fn materialize_membership_snapshot(
        &self,
        project_slug: &str,
        members: &[(String, String)],
        provider_instance: &str,
        ts: u64,
    ) -> Result<()> {
        // Legacy table: authoritative wholesale replace.
        self.replace_group_members(project_slug, members, ts)?;
        // Canonical mirror via Phase 1 accessor.
        const FABRIC: &str = "kind1-nip29";
        if let Some(pid) = self.project_id_for_origin(FABRIC, provider_instance, project_slug)? {
            for (pubkey, role) in members {
                self.admit_member(&pid, pubkey, role, "nip29-39002", ts)?;
            }
        }
        Ok(())
    }

    /// Record an inbound mention in the legacy inbox with full dedup semantics.
    /// Returns true when the row was newly stored.
    ///
    /// Canonical `messages` dual-write comes in Phase 6.
    pub fn materialize_inbound_message(&self, m: &InboxRow) -> Result<bool> {
        self.enqueue_mention(m)
    }

    /// Record an outbound message in the canonical `messages` table.
    /// Returns the `message_id`.
    pub fn materialize_outbound_message(
        &self,
        thread_id: &str,
        author_pubkey: &str,
        body: &str,
        created_at: u64,
        native_event_id: Option<&str>,
    ) -> Result<String> {
        self.record_message(
            thread_id,
            author_pubkey,
            body,
            created_at,
            "outbound",
            "pending",
            native_event_id,
        )
    }

    /// Transition an outbound message to `accepted` (relay accepted the event).
    pub fn mark_outbound_accepted(&self, message_id: &str) -> Result<()> {
        self.mark_message_sync_state(message_id, "accepted", None)
    }

    /// Transition an outbound message to `echoed` (relay echoed it back to us).
    pub fn mark_outbound_echoed(&self, message_id: &str) -> Result<()> {
        self.mark_message_sync_state(message_id, "echoed", None)
    }

    /// Transition an outbound message to `failed` with an error string.
    pub fn mark_outbound_failed(&self, message_id: &str, error: &str) -> Result<()> {
        self.mark_message_sync_state(message_id, "failed", Some(error))
    }
}

mod endpoints;
pub use endpoints::SessionEndpoint;

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

/// Column order: mention_event_id, target_session, from_pubkey, from_slug,
/// project, body, created_at, from_session, subject, branch, commit_hash, dirty,
/// host. Shared by `peek_inbox`, `drain_inbox`, and `find_inbox_by_event_prefix`.
fn row_to_inbox(row: &rusqlite::Row) -> rusqlite::Result<InboxRow> {
    Ok(InboxRow {
        mention_event_id: row.get(0)?,
        target_session: row.get(1)?,
        from_pubkey: row.get(2)?,
        from_slug: row.get(3)?,
        project: row.get(4)?,
        body: row.get(5)?,
        created_at: row.get(6)?,
        from_session: row.get(7)?,
        subject: row.get(8)?,
        branch: row.get(9)?,
        commit: row.get(10)?,
        dirty: row.get::<_, i64>(11)? as u32,
        host: row.get(12)?,
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
            subject: String::new(),
            branch: String::new(),
            commit: String::new(),
            dirty: 0,
            host: String::new(),
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
            s.latest_alive_session_for_project("proj")
                .unwrap()
                .unwrap()
                .agent_slug,
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
        s.upsert_peer_session(
            "sess-x",
            "pk-from-presence",
            "reviewer",
            "proj",
            "host",
            "",
            1,
        )
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
            s.resolve_agent_pubkey("reviewer", None).unwrap().as_deref(),
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
        assert_eq!(
            s.list_peer_sessions(Some("proj"), 0).unwrap()[0].rel_cwd,
            "sub/dir"
        );

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
        s.set_agent_status("pk-a", "current", None, "working here", "", true, 100)
            .unwrap();
        s.set_agent_status("pk-b", "elsewhere", None, "working there", "", true, 100)
            .unwrap();

        let scoped = s.list_status_changes_since(50, Some("current")).unwrap();
        assert_eq!(
            scoped,
            vec![(
                "alpha".to_string(),
                "current".to_string(),
                "working here".to_string(),
                None,
                true
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
        s.upsert_group_member("proj", "pk-a", "member", 100)
            .unwrap();
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
        s.upsert_group_member("proj", "stale", "member", 100)
            .unwrap();
        // A relay 39002 snapshot replaces the whole set: 'stale' drops out.
        s.replace_group_members(
            "proj",
            &[
                ("pk-a".into(), "member".into()),
                ("pk-b".into(), "admin".into()),
            ],
            300,
        )
        .unwrap();
        assert!(!s.is_group_member("proj", "stale").unwrap());
        assert!(s.is_group_member("proj", "pk-a").unwrap());
        assert!(s.is_group_member("proj", "pk-b").unwrap());
        // Scoped to the project — a different group is untouched.
        s.upsert_group_member("other", "pk-x", "member", 100)
            .unwrap();
        s.replace_group_members("proj", &[], 400).unwrap();
        assert!(!s.is_group_member("proj", "pk-a").unwrap());
        assert!(s.is_group_member("other", "pk-x").unwrap());
    }

    // ── freeze tests (Phase-0 regression oracle) ─────────────────────────────

    /// FREEZE B1: enqueue_mention is idempotent on (mention_event_id, target_session).
    /// Recording the same event id for the same session twice yields exactly one row;
    /// the second call returns false. A different session with the same event id is
    /// a DISTINCT row (different PK component).
    #[test]
    fn freeze_inbox_dedup_by_event_id() {
        let s = Store::open_memory().unwrap();
        let base = InboxRow {
            mention_event_id: "evt-freeze-1".into(),
            target_session: "sess-X".into(),
            from_pubkey: "pk-sender".into(),
            from_slug: "sender".into(),
            project: "proj".into(),
            body: "hello".into(),
            created_at: 100,
            from_session: "".into(),
            subject: String::new(),
            branch: String::new(),
            commit: String::new(),
            dirty: 0,
            host: String::new(),
        };

        // First insert: new row → true.
        assert!(
            s.enqueue_mention(&base).unwrap(),
            "first insert must return true"
        );

        // Duplicate for the SAME (event_id, session): must be ignored → false.
        assert!(
            !s.enqueue_mention(&base).unwrap(),
            "duplicate must be ignored (idempotent)"
        );

        // Same event id, DIFFERENT session: distinct PK → separate delivery → true.
        let mut other_session = base.clone();
        other_session.target_session = "sess-Y".into();
        assert!(
            s.enqueue_mention(&other_session).unwrap(),
            "same event_id, different session = distinct row"
        );

        // Both sessions have exactly one undelivered row.
        assert_eq!(s.peek_inbox("sess-X").unwrap().len(), 1);
        assert_eq!(s.peek_inbox("sess-Y").unwrap().len(), 1);

        // drain_inbox marks delivered; a second drain is empty.
        let drained = s.drain_inbox("sess-X").unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].body, "hello");
        assert!(
            s.drain_inbox("sess-X").unwrap().is_empty(),
            "delivered rows must not re-drain"
        );
    }

    /// FREEZE B2: replace_group_members applied TWICE with the same snapshot is
    /// idempotent — no duplicates, no stale survivors, and other projects are
    /// unaffected. This extends the existing authoritative-replace test.
    #[test]
    fn freeze_replace_group_members_idempotent_re_apply() {
        let s = Store::open_memory().unwrap();
        let snapshot: Vec<(String, String)> = vec![
            ("pk-alpha".into(), "member".into()),
            ("pk-beta".into(), "admin".into()),
        ];

        // Seed a stale member that should vanish.
        s.upsert_group_member("proj", "pk-stale", "member", 50)
            .unwrap();

        // First apply.
        s.replace_group_members("proj", &snapshot, 200).unwrap();
        assert!(s.is_group_member("proj", "pk-alpha").unwrap());
        assert!(s.is_group_member("proj", "pk-beta").unwrap());
        assert!(!s.is_group_member("proj", "pk-stale").unwrap());

        // Identical second apply — observable membership must be unchanged.
        s.replace_group_members("proj", &snapshot, 300).unwrap();
        assert!(
            s.is_group_member("proj", "pk-alpha").unwrap(),
            "alpha still member after re-apply"
        );
        assert!(
            s.is_group_member("proj", "pk-beta").unwrap(),
            "beta still member after re-apply"
        );
        assert!(
            !s.is_group_member("proj", "pk-stale").unwrap(),
            "stale still absent after re-apply"
        );

        // A sibling project is completely unaffected by both applies.
        s.upsert_group_member("other-proj", "pk-other", "member", 100)
            .unwrap();
        s.replace_group_members("proj", &snapshot, 400).unwrap();
        assert!(
            s.is_group_member("other-proj", "pk-other").unwrap(),
            "sibling project untouched"
        );
        assert!(!s.is_group_member("other-proj", "pk-alpha").unwrap());
    }

    /// FREEZE B4: peek_inbox is read-only — rows survive a peek and remain
    /// available to drain. drain_inbox marks them delivered.
    #[test]
    fn freeze_peek_is_nondestructive_drain_is_final() {
        let s = Store::open_memory().unwrap();
        let row = InboxRow {
            mention_event_id: "evt-peek-1".into(),
            target_session: "sess-peek".into(),
            from_pubkey: "pk-s".into(),
            from_slug: "sender".into(),
            project: "proj".into(),
            body: "peek me".into(),
            created_at: 1,
            from_session: "".into(),
            subject: String::new(),
            branch: String::new(),
            commit: String::new(),
            dirty: 0,
            host: String::new(),
        };
        s.enqueue_mention(&row).unwrap();

        // peek: row is visible.
        assert_eq!(
            s.peek_inbox("sess-peek").unwrap().len(),
            1,
            "peek must see the row"
        );
        // peek again: still there (not consumed).
        assert_eq!(
            s.peek_inbox("sess-peek").unwrap().len(),
            1,
            "second peek must still see the row"
        );

        // drain: consumes and marks delivered.
        let drained = s.drain_inbox("sess-peek").unwrap();
        assert_eq!(drained.len(), 1);

        // After drain, both peek and drain return empty.
        assert!(
            s.peek_inbox("sess-peek").unwrap().is_empty(),
            "peek after drain must be empty"
        );
        assert!(
            s.drain_inbox("sess-peek").unwrap().is_empty(),
            "second drain must be empty"
        );
    }

    // ── Phase 1: canonical read-model schema ─────────────────────────────

    #[test]
    fn phase1_new_tables_exist_after_open() {
        let s = Store::open_memory().unwrap();
        let n: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                 ('projects','project_origins','threads','thread_origins','messages',
                  'message_recipients','inbound_quarantine','membership')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 8, "all 8 Phase 1 tables must be created");
    }

    #[test]
    fn phase1_ensure_project_origin_is_idempotent() {
        let s = Store::open_memory().unwrap();
        let a = s
            .ensure_project_origin("kind1-nip29", "relayhash", "tenex-edge", "tenex-edge", 100)
            .unwrap();
        let b = s
            .ensure_project_origin("kind1-nip29", "relayhash", "tenex-edge", "tenex-edge", 200)
            .unwrap();
        assert_eq!(a, b, "same origin → same project_id");
        let count: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "no duplicate project row");
        assert_eq!(
            s.project_id_for_origin("kind1-nip29", "relayhash", "tenex-edge")
                .unwrap(),
            Some(a.clone())
        );
        // A different fabric/instance/key is a distinct project.
        let c = s
            .ensure_project_origin("kind1-nip29", "relayhash", "other", "other", 100)
            .unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn phase1_is_member_at_lifecycle() {
        let s = Store::open_memory().unwrap();
        let pid = s
            .ensure_project_origin("kind1-nip29", "ri", "p", "p", 10)
            .unwrap();
        // No membership rows at all → Unhydrated.
        assert_eq!(
            s.is_member_at(&pid, "alice", 100).unwrap(),
            MembershipDecision::Unhydrated
        );
        // Admit bob → bob is Member, alice is NotMember (rows now exist).
        s.admit_member(&pid, "bob", "member", "nip29-39002", 50)
            .unwrap();
        assert_eq!(
            s.is_member_at(&pid, "bob", 100).unwrap(),
            MembershipDecision::Member {
                role: "member".into()
            }
        );
        assert_eq!(
            s.is_member_at(&pid, "alice", 100).unwrap(),
            MembershipDecision::NotMember
        );
        // A query before bob's admission time sees him as not-yet-member.
        assert_eq!(
            s.is_member_at(&pid, "bob", 40).unwrap(),
            MembershipDecision::NotMember
        );
        // Revoke bob at t=80 → Revoked when queried at/after 80, still Member before.
        s.revoke_member(&pid, "bob", 80).unwrap();
        assert_eq!(
            s.is_member_at(&pid, "bob", 100).unwrap(),
            MembershipDecision::Revoked
        );
        assert_eq!(
            s.is_member_at(&pid, "bob", 60).unwrap(),
            MembershipDecision::Member {
                role: "member".into()
            }
        );
        // Re-admit clears the revocation.
        s.admit_member(&pid, "bob", "admin", "nip29-39002", 90)
            .unwrap();
        assert_eq!(
            s.is_member_at(&pid, "bob", 100).unwrap(),
            MembershipDecision::Member {
                role: "admin".into()
            }
        );
    }

    #[test]
    fn phase1_record_message_dedups_on_native_event_id() {
        let s = Store::open_memory().unwrap();
        let pid = s
            .ensure_project_origin("kind1-nip29", "ri", "p", "p", 1)
            .unwrap();
        let tid = s
            .ensure_thread_origin(&pid, "kind1-nip29", "ri", "root-eid", 1)
            .unwrap();
        let m1 = s
            .record_message(
                &tid,
                "author",
                "hi",
                10,
                "inbound",
                "accepted",
                Some("evt-1"),
            )
            .unwrap();
        let m2 = s
            .record_message(
                &tid,
                "author",
                "hi (echo)",
                10,
                "inbound",
                "accepted",
                Some("evt-1"),
            )
            .unwrap();
        assert_eq!(m1, m2, "same native_event_id → same message_id (no dup)");
        let count: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        // None native id always inserts a fresh row.
        let m3 = s
            .record_message(&tid, "author", "local", 11, "outbound", "published", None)
            .unwrap();
        assert_ne!(m1, m3);
        // Recipient rows are idempotent.
        s.add_message_recipient(&m1, "rcpt", Some("sess-1"))
            .unwrap();
        s.add_message_recipient(&m1, "rcpt", Some("sess-1"))
            .unwrap();
        let rc: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM message_recipients", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rc, 1);
    }

    #[test]
    fn phase1_quarantine_roundtrip_and_idempotent() {
        let s = Store::open_memory().unwrap();
        s.quarantine_inbound("evt-q", Some("proj-x"), "unhydrated", "{\"raw\":1}", 5)
            .unwrap();
        s.quarantine_inbound("evt-q", Some("proj-x"), "unhydrated", "{\"raw\":1}", 9)
            .unwrap();
        let all = s.replay_quarantine(None).unwrap();
        assert_eq!(all.len(), 1, "INSERT OR IGNORE dedups by native_event_id");
        assert_eq!(all[0].project_id.as_deref(), Some("proj-x"));
        assert!(s.replay_quarantine(Some("nope")).unwrap().is_empty());
        assert_eq!(s.replay_quarantine(Some("proj-x")).unwrap().len(), 1);
        s.clear_quarantine("evt-q").unwrap();
        assert!(s.replay_quarantine(None).unwrap().is_empty());
    }

    #[test]
    fn phase1_backfill_is_idempotent() {
        let s = Store::open_memory().unwrap();
        // Seed legacy state across the four source tables.
        s.upsert_project_meta("tenex-edge", "the edge fabric", 1)
            .unwrap();
        s.upsert_peer_session("ps-1", "pk-peer", "peer", "otherproj", "host", "", 1)
            .unwrap();
        s.replace_group_members(
            "tenex-edge",
            &[
                ("pk-1".into(), "admin".into()),
                ("pk-2".into(), "member".into()),
            ],
            1,
        )
        .unwrap();

        let projects_before = || -> i64 {
            s.conn
                .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
                .unwrap()
        };
        let members_before = || -> i64 {
            s.conn
                .query_row("SELECT COUNT(*) FROM membership", [], |r| r.get(0))
                .unwrap()
        };

        s.backfill_kind1_nip29_origins("relayhash", 100).unwrap();
        let p1 = projects_before();
        let m1 = members_before();
        assert!(p1 >= 2, "tenex-edge + otherproj origins created (got {p1})");
        assert_eq!(m1, 2, "two group_members mirrored into membership");

        // about carried from project_meta onto the canonical project row.
        let pid = s
            .project_id_for_origin("kind1-nip29", "relayhash", "tenex-edge")
            .unwrap()
            .unwrap();
        let about: Option<String> = s
            .conn
            .query_row(
                "SELECT about FROM projects WHERE project_id=?1",
                params![pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(about.as_deref(), Some("the edge fabric"));

        // membership reflects the roster.
        assert_eq!(
            s.is_member_at(&pid, "pk-1", 200).unwrap(),
            MembershipDecision::Member {
                role: "admin".into()
            }
        );

        // Second run is a no-op at the row-count level.
        s.backfill_kind1_nip29_origins("relayhash", 300).unwrap();
        assert_eq!(
            projects_before(),
            p1,
            "no duplicate project rows on re-backfill"
        );
        assert_eq!(
            members_before(),
            m1,
            "no duplicate membership rows on re-backfill"
        );
    }

    // ── Phase 2: read-model and write-facing materializer unit tests ─────────

    /// list_projects_read_model delegates to list_project_meta — same rows, same order.
    #[test]
    fn phase2_list_projects_read_model_matches_project_meta() {
        let s = Store::open_memory().unwrap();
        assert!(s.list_projects_read_model().unwrap().is_empty());
        s.upsert_project_meta("zap", "about-zap", 1).unwrap();
        s.upsert_project_meta("alpha", "about-alpha", 2).unwrap();
        let rows = s.list_projects_read_model().unwrap();
        // list_project_meta orders by project slug.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "alpha");
        assert_eq!(rows[1].0, "zap");
        assert_eq!(rows[0].1, "about-alpha");
    }

    /// project_meta_read_model is a pass-through of get_project_meta.
    #[test]
    fn phase2_project_meta_read_model_passthrough() {
        let s = Store::open_memory().unwrap();
        assert!(s.project_meta_read_model("missing").unwrap().is_none());
        s.upsert_project_meta("proj", "the about", 1).unwrap();
        assert_eq!(
            s.project_meta_read_model("proj").unwrap().as_deref(),
            Some("the about")
        );
    }

    /// list_agents_read_model returns alive sessions filtered by project + freshness.
    #[test]
    fn phase2_list_agents_read_model_filters() {
        let s = Store::open_memory().unwrap();
        let mut r = sample_session("s1");
        r.project = "proj".into();
        s.upsert_session(&r).unwrap();
        s.touch_session("s1", 1000).unwrap();

        let mut r2 = sample_session("s2");
        r2.project = "other".into();
        s.upsert_session(&r2).unwrap();
        s.touch_session("s2", 1000).unwrap();

        // Project-scoped.
        let proj = s.list_agents_read_model(Some("proj"), 0).unwrap();
        assert_eq!(proj.len(), 1);
        assert_eq!(proj[0].session_id, "s1");

        // Freshness filter: since=1001 → both stale.
        assert!(s.list_agents_read_model(None, 1001).unwrap().is_empty());

        // All projects, no freshness filter.
        assert_eq!(s.list_agents_read_model(None, 0).unwrap().len(), 2);
    }

    /// list_presence_read_model delegates to list_peer_sessions.
    #[test]
    fn phase2_list_presence_read_model_delegates() {
        let s = Store::open_memory().unwrap();
        s.upsert_peer_session("ps1", "pk-a", "agentA", "proj", "host", "", 500)
            .unwrap();
        let rows = s.list_presence_read_model(Some("proj"), 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slug, "agentA");
        // Since filter.
        assert!(s
            .list_presence_read_model(Some("proj"), 600)
            .unwrap()
            .is_empty());
    }

    /// list_status_read_model returns (pubkey, project, text) rows.
    #[test]
    fn phase2_list_status_read_model() {
        let s = Store::open_memory().unwrap();
        s.set_agent_status("pk-a", "proj", None, "working", "", true, 100)
            .unwrap();
        s.set_agent_status("pk-b", "other", None, "idle", "", false, 200)
            .unwrap();
        let all = s.list_status_read_model(None).unwrap();
        assert_eq!(all.len(), 2);
        let scoped = s.list_status_read_model(Some("proj")).unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].0, "pk-a");
        assert_eq!(scoped[0].2, "working");
    }

    /// list_threads returns empty on a fresh store (canonical table, Phase 7).
    #[test]
    fn phase2_list_threads_empty_until_phase7() {
        let s = Store::open_memory().unwrap();
        let pid = s
            .ensure_project_origin("kind1-nip29", "ri", "p", "p", 1)
            .unwrap();
        assert!(
            s.list_threads(&pid).unwrap().is_empty(),
            "threads empty before Phase 7"
        );
        // After ensure_thread_origin it is populated — verify the enriched struct.
        let tid = s
            .ensure_thread_origin(&pid, "kind1-nip29", "ri", "t1", 2)
            .unwrap();
        let threads = s.list_threads(&pid).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, tid);
        assert_eq!(threads[0].project_id, pid);
        assert_eq!(threads[0].message_count, 0);
        assert!(threads[0].last_message_at.is_none());
    }

    /// messages_for_thread returns empty on a fresh thread (canonical table, Phase 6).
    #[test]
    fn phase2_messages_for_thread_empty_until_phase6() {
        let s = Store::open_memory().unwrap();
        let pid = s
            .ensure_project_origin("kind1-nip29", "ri", "p", "p", 1)
            .unwrap();
        let tid = s
            .ensure_thread_origin(&pid, "kind1-nip29", "ri", "t1", 2)
            .unwrap();
        assert!(s.messages_for_thread(&tid).unwrap().is_empty());
        let mid = s
            .record_message(&tid, "pk", "hello", 3, "inbound", "accepted", None)
            .unwrap();
        let msgs = s.messages_for_thread(&tid).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].message_id, mid);
        assert_eq!(msgs[0].body, "hello");
    }

    /// undelivered_messages_for_session is non-destructive (same as peek_inbox).
    #[test]
    fn phase2_undelivered_messages_for_session_is_nondestructive() {
        let s = Store::open_memory().unwrap();
        let row = InboxRow {
            mention_event_id: "evt-rdm-1".into(),
            target_session: "sess-rm".into(),
            from_pubkey: "pk-s".into(),
            from_slug: "sender".into(),
            project: "proj".into(),
            body: "hello rdm".into(),
            created_at: 1,
            from_session: "".into(),
            subject: String::new(),
            branch: String::new(),
            commit: String::new(),
            dirty: 0,
            host: String::new(),
        };
        s.enqueue_mention(&row).unwrap();
        // Call twice — rows survive (non-destructive).
        assert_eq!(
            s.undelivered_messages_for_session("sess-rm").unwrap().len(),
            1
        );
        assert_eq!(
            s.undelivered_messages_for_session("sess-rm").unwrap().len(),
            1
        );
        // drain_inbox still works after peeking via the read-model method.
        let drained = s.drain_inbox("sess-rm").unwrap();
        assert_eq!(drained.len(), 1);
        assert!(s
            .undelivered_messages_for_session("sess-rm")
            .unwrap()
            .is_empty());
    }

    /// materialize_profile round-trips through upsert_profile.
    #[test]
    fn phase2_materialize_profile() {
        let s = Store::open_memory().unwrap();
        s.materialize_profile("pk-mp", "agent-mp", "host-mp", 100)
            .unwrap();
        let pk = s.resolve_agent_pubkey("agent-mp", None).unwrap();
        assert_eq!(pk.as_deref(), Some("pk-mp"));
    }

    /// materialize_presence round-trips through upsert_peer_session.
    #[test]
    fn phase2_materialize_presence() {
        let s = Store::open_memory().unwrap();
        s.materialize_presence(
            "sess-mp", "pk-mp", "agent-mp", "proj", "host", "subdir", 100,
        )
        .unwrap();
        let rows = s.list_presence_read_model(Some("proj"), 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rel_cwd, "subdir");
    }

    /// materialize_status round-trips through set_agent_status.
    #[test]
    fn phase2_materialize_status() {
        let s = Store::open_memory().unwrap();
        s.materialize_status(
            "pk-ms",
            "proj",
            None,
            "reviewing",
            "reading the diff",
            true,
            100,
        )
        .unwrap();
        assert_eq!(
            s.get_agent_status("pk-ms", "proj", None).unwrap(),
            Some((
                "reviewing".to_string(),
                "reading the diff".to_string(),
                true
            ))
        );
    }

    /// materialize_membership_snapshot replaces legacy group_members AND mirrors
    /// into canonical membership when a project origin already exists.
    #[test]
    fn phase2_materialize_membership_snapshot_updates_both_tables() {
        let s = Store::open_memory().unwrap();
        // Seed a legacy stale member.
        s.upsert_group_member("proj", "stale", "member", 50)
            .unwrap();
        // Seed canonical origin.
        let pid = s
            .ensure_project_origin("kind1-nip29", "ri", "proj", "proj", 1)
            .unwrap();

        let members = vec![
            ("pk-a".to_string(), "member".to_string()),
            ("pk-b".to_string(), "admin".to_string()),
        ];
        s.materialize_membership_snapshot("proj", &members, "ri", 200)
            .unwrap();

        // Legacy table: stale gone, new members present.
        assert!(!s.is_group_member("proj", "stale").unwrap());
        assert!(s.is_group_member("proj", "pk-a").unwrap());
        assert!(s.is_group_member("proj", "pk-b").unwrap());

        // Canonical membership mirrored.
        assert_eq!(
            s.is_member_at(&pid, "pk-a", 300).unwrap(),
            MembershipDecision::Member {
                role: "member".into()
            }
        );
        assert_eq!(
            s.is_member_at(&pid, "pk-b", 300).unwrap(),
            MembershipDecision::Member {
                role: "admin".into()
            }
        );
    }

    /// materialize_membership_snapshot still updates legacy even without a canonical origin.
    #[test]
    fn phase2_materialize_membership_no_origin_still_updates_legacy() {
        let s = Store::open_memory().unwrap();
        let members = vec![("pk-x".to_string(), "member".to_string())];
        // No project_origins row → canonical mirror is a no-op, legacy still updates.
        s.materialize_membership_snapshot("unknown-proj", &members, "ri", 200)
            .unwrap();
        assert!(s.is_group_member("unknown-proj", "pk-x").unwrap());
    }

    /// materialize_inbound_message is idempotent (delegates to enqueue_mention).
    #[test]
    fn phase2_materialize_inbound_message_idempotent() {
        let s = Store::open_memory().unwrap();
        let row = InboxRow {
            mention_event_id: "evt-mat-1".into(),
            target_session: "sess-mat".into(),
            from_pubkey: "pk-s".into(),
            from_slug: "sender".into(),
            project: "proj".into(),
            body: "inbound".into(),
            created_at: 1,
            from_session: "".into(),
            subject: String::new(),
            branch: String::new(),
            commit: String::new(),
            dirty: 0,
            host: String::new(),
        };
        assert!(
            s.materialize_inbound_message(&row).unwrap(),
            "first insert → true"
        );
        assert!(
            !s.materialize_inbound_message(&row).unwrap(),
            "duplicate → false (idempotent)"
        );
        assert_eq!(s.peek_inbox("sess-mat").unwrap().len(), 1);
    }

    /// materialize_outbound_message, mark_outbound_accepted/echoed/failed
    /// round-trip through the canonical messages table.
    #[test]
    fn phase2_materialize_outbound_lifecycle() {
        let s = Store::open_memory().unwrap();
        let pid = s
            .ensure_project_origin("kind1-nip29", "ri", "p", "p", 1)
            .unwrap();
        let tid = s
            .ensure_thread_origin(&pid, "kind1-nip29", "ri", "t1", 2)
            .unwrap();

        let mid = s
            .materialize_outbound_message(&tid, "pk-author", "hey", 10, Some("nat-1"))
            .unwrap();
        // Initial state is "pending".
        let state: String = s
            .conn
            .query_row(
                "SELECT sync_state FROM messages WHERE message_id=?1",
                params![mid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "pending");

        s.mark_outbound_accepted(&mid).unwrap();
        let state: String = s
            .conn
            .query_row(
                "SELECT sync_state FROM messages WHERE message_id=?1",
                params![mid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "accepted");

        s.mark_outbound_echoed(&mid).unwrap();
        let state: String = s
            .conn
            .query_row(
                "SELECT sync_state FROM messages WHERE message_id=?1",
                params![mid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "echoed");

        s.mark_outbound_failed(&mid, "relay rejected").unwrap();
        let (st, err): (String, Option<String>) = s
            .conn
            .query_row(
                "SELECT sync_state, error FROM messages WHERE message_id=?1",
                params![mid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(st, "failed");
        assert_eq!(err.as_deref(), Some("relay rejected"));

        // Idempotent dedup on native_event_id.
        let mid2 = s
            .materialize_outbound_message(&tid, "pk-author", "hey (echo)", 10, Some("nat-1"))
            .unwrap();
        assert_eq!(mid, mid2, "same native_event_id → same message_id");
    }

    // ── Phase 6 dual-write tests ──────────────────────────────────────────────

    /// Regression (found via live claude<->codex e2e): an UNTARGETED recipient
    /// (target_session = None) must be idempotent. SQLite treats NULL as distinct
    /// in the PK, so a naive INSERT OR IGNORE accumulated one duplicate recipient
    /// row per re-materialization (relay echo + every catch-up refetch).
    #[test]
    fn add_message_recipient_is_idempotent_for_null_target_session() {
        let s = Store::open_memory().unwrap();
        let tid = s
            .ensure_thread_origin(
                &s.ensure_project_origin("kind1-nip29", "pi", "p", "p", 1)
                    .unwrap(),
                "kind1-nip29",
                "pi",
                "root",
                1,
            )
            .unwrap();
        let mid = s
            .record_message(&tid, "auth", "b", 1, "inbound", "received", Some("evt"))
            .unwrap();
        // Untargeted: many re-deliveries → still one row.
        for _ in 0..5 {
            s.add_message_recipient(&mid, "rcpt", None).unwrap();
        }
        let n: i64 = s.conn.query_row(
            "SELECT COUNT(*) FROM message_recipients WHERE message_id=?1 AND recipient_pubkey='rcpt' AND target_session IS NULL",
            params![mid], |r| r.get(0),
        ).unwrap();
        assert_eq!(
            n, 1,
            "untargeted recipient must not duplicate across re-materialization"
        );
        // Targeted dedup still works, and is a DISTINCT row from the untargeted one.
        for _ in 0..3 {
            s.add_message_recipient(&mid, "rcpt", Some("sess-1"))
                .unwrap();
        }
        let total: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM message_recipients WHERE message_id=?1",
                params![mid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(total, 2, "one untargeted + one targeted recipient row");
    }

    /// Phase 6 outbound dual-write: the canonical row sequence used by
    /// `provider.send()` produces exactly one message with sync_state="published",
    /// one recipient row, and is idempotent on native_event_id (relay echo).
    #[test]
    fn phase6_outbound_canonical_dual_write_and_dedup() {
        let s = Store::open_memory().unwrap();
        let pi = "test-pi";
        let now = 1000u64;
        let eid = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";

        // Simulate what provider.send() does after publish succeeds.
        let project_id = s
            .ensure_project_origin("kind1-nip29", pi, "my-project", "my-project", now)
            .unwrap();
        let thread_id = s
            .ensure_thread_origin(&project_id, "kind1-nip29", pi, eid, now)
            .unwrap();
        let message_id = s
            .record_message(
                &thread_id,
                "pk-sender",
                "hello world",
                now,
                "outbound",
                "published",
                Some(eid),
            )
            .unwrap();
        s.add_message_recipient(&message_id, "pk-recipient", Some("sess-r1"))
            .unwrap();

        // Verify the canonical message row.
        let (direction, sync_state, native_eid): (String, String, Option<String>) = s
            .conn
            .query_row(
                "SELECT direction, sync_state, native_event_id FROM messages WHERE message_id=?1",
                params![message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(direction, "outbound");
        assert_eq!(sync_state, "published");
        assert_eq!(native_eid.as_deref(), Some(eid));

        // Verify exactly one recipient row.
        let rcpt_count: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM message_recipients WHERE message_id=?1",
                params![message_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rcpt_count, 1, "exactly one recipient row");

        // Idempotency: same native_event_id → same message_id, no new row.
        let mid2 = s
            .record_message(
                &thread_id,
                "pk-sender",
                "hello world (echo)",
                now,
                "outbound",
                "published",
                Some(eid),
            )
            .unwrap();
        assert_eq!(
            message_id, mid2,
            "same native_event_id → same message_id (dedup)"
        );

        // add_message_recipient is INSERT OR IGNORE → still only one row.
        s.add_message_recipient(&message_id, "pk-recipient", Some("sess-r1"))
            .unwrap();
        let rcpt_count2: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM message_recipients WHERE message_id=?1",
                params![message_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rcpt_count2, 1, "add_message_recipient is idempotent");
    }

    /// Phase 6 inbound dual-write: materializing the same inbound event twice
    /// (simulating relay echo) yields exactly one canonical message row, one
    /// recipient row, and exactly one legacy inbox row.
    #[test]
    fn phase6_inbound_canonical_dual_write_and_dedup() {
        use crate::fabric::kind1::materializer::Kind1Materializer;
        use nostr_sdk::prelude::{EventBuilder, Keys, Kind};

        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pk_hex = keys.public_key().to_hex();

        // Create a recipient session so the legacy inbox path has somewhere to deliver.
        let rec = SessionRecord {
            session_id: "sess-inbound-1".into(),
            agent_slug: "agent".into(),
            agent_pubkey: pk_hex.clone(),
            project: "test-proj".into(),
            host: "host".into(),
            child_pid: None,
            watch_pid: None,
            created_at: 1,
            alive: true,
            rel_cwd: String::new(),
        };
        s.upsert_session(&rec).unwrap();

        // Build a signed Event (kind:1) that looks like a mention.
        // We use a minimal event — the codec is not involved here;
        // we test the materialize_inbound_message store writes directly.
        let sender_keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(1u16), "hi from sender")
            .sign_with_keys(&sender_keys)
            .unwrap();
        let eid = event.id.to_hex();

        let mention = crate::domain::Mention {
            from: crate::domain::AgentRef::new(
                sender_keys.public_key().to_hex(),
                "sender".to_string(),
            ),
            to_pubkey: pk_hex.clone(),
            project: "test-proj".into(),
            body: "hi from sender".into(),
            target_session: Some("sess-inbound-1".into()),
            from_session: None,
            meta: crate::domain::MentionMeta::default(),
        };
        let pi = "test-pi-inbound";
        let now = 2000u64;

        // First materialization.
        let (routed1, thread1) =
            Kind1Materializer::materialize_inbound_message(&s, &pk_hex, &mention, &event, pi, now);
        assert!(routed1, "first delivery must route to session");
        assert!(
            thread1.is_some(),
            "canonical write must report the thread id"
        );

        // Second materialization (relay echo) — must be a no-op everywhere.
        let (routed2, _) =
            Kind1Materializer::materialize_inbound_message(&s, &pk_hex, &mention, &event, pi, now);
        assert!(
            !routed2,
            "echo: inbox already has this (mention_event_id, target_session)"
        );

        // Exactly one canonical message row.
        let msg_count: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE native_event_id=?1",
                params![eid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(msg_count, 1, "exactly one canonical message after echo");

        // Exactly one recipient row.
        let mid: String = s
            .conn
            .query_row(
                "SELECT message_id FROM messages WHERE native_event_id=?1",
                params![eid],
                |r| r.get(0),
            )
            .unwrap();
        let rcpt_count: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM message_recipients WHERE message_id=?1",
                params![mid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rcpt_count, 1, "exactly one recipient row after echo");

        // Exactly one legacy inbox row (the dedup the legacy path enforces).
        let inbox_count: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM inbox WHERE mention_event_id=?1 AND target_session='sess-inbound-1'",
                params![eid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(inbox_count, 1, "exactly one legacy inbox row after echo");
    }

    // ── Phase 7 tests ─────────────────────────────────────────────────────────

    /// list_threads/messages_for_thread/thread_meta return correct enriched data.
    #[test]
    fn phase7_read_model_enriched_data() {
        let s = Store::open_memory().unwrap();
        let pi = "test-pi-p7";
        let pid = s
            .ensure_project_origin("kind1-nip29", pi, "myproj", "myproj", 100)
            .unwrap();

        // Two threads; second thread has messages; first does not.
        let tid1 = s
            .ensure_thread_origin(&pid, "kind1-nip29", pi, "native-t1", 100)
            .unwrap();
        let tid2 = s
            .ensure_thread_origin(&pid, "kind1-nip29", pi, "native-t2", 200)
            .unwrap();

        // Add two messages to tid2.
        let _m1 = s
            .record_message(
                &tid2,
                "pk-a",
                "first",
                300,
                "inbound",
                "received",
                Some("eid-1"),
            )
            .unwrap();
        let _m2 = s
            .record_message(
                &tid2,
                "pk-b",
                "second",
                400,
                "outbound",
                "published",
                Some("eid-2"),
            )
            .unwrap();

        // list_threads — tid2 (last activity 400) should come first.
        let threads = s.list_threads(&pid).unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].thread_id, tid2, "most-active thread is first");
        assert_eq!(threads[0].message_count, 2);
        assert_eq!(threads[0].last_message_at, Some(400));
        assert_eq!(threads[1].thread_id, tid1, "inactive thread is second");
        assert_eq!(threads[1].message_count, 0);
        assert!(threads[1].last_message_at.is_none());

        // messages_for_thread — chronological order.
        let msgs = s.messages_for_thread(&tid2).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].body, "first");
        assert_eq!(msgs[0].direction, "inbound");
        assert_eq!(msgs[1].body, "second");
        assert_eq!(msgs[1].direction, "outbound");
        assert_eq!(msgs[1].native_event_id.as_deref(), Some("eid-2"));

        // thread_meta — single thread lookup.
        let meta = s.thread_meta(&tid2).unwrap().expect("thread_meta found");
        assert_eq!(meta.thread_id, tid2);
        assert_eq!(meta.message_count, 2);
        assert_eq!(meta.last_message_at, Some(400));

        // thread_meta on a non-existent id returns None.
        assert!(s.thread_meta("no-such-thread").unwrap().is_none());
    }

    /// thread_root_native_key resolves the relay-native key for a thread origin.
    #[test]
    fn phase7_thread_root_native_key() {
        let s = Store::open_memory().unwrap();
        let pi = "pi-rootkey";
        let pid = s
            .ensure_project_origin("kind1-nip29", pi, "proj", "proj", 1)
            .unwrap();
        let tid = s
            .ensure_thread_origin(&pid, "kind1-nip29", pi, "root-event-abc", 2)
            .unwrap();

        let key = s.thread_root_native_key(&tid, "kind1-nip29", pi);
        assert_eq!(key.as_deref(), Some("root-event-abc"));

        // Wrong fabric or provider_instance → None.
        assert!(s.thread_root_native_key(&tid, "other-fabric", pi).is_none());
        assert!(s
            .thread_root_native_key(&tid, "kind1-nip29", "wrong-pi")
            .is_none());
    }

    /// Reply grouping round-trip: a reply event carrying a root `e` tag pointing
    /// at an existing outbound thread's native key lands in the SAME thread.
    ///
    /// The Phase 6 inbound materializer reads `["e", ..., "root"]` to determine
    /// `native_thread_key`.  This test proves the end-to-end grouping works.
    #[test]
    fn phase7_reply_groups_into_same_thread() {
        use crate::domain::{AgentRef, Mention};
        use crate::fabric::kind1::materializer::Kind1Materializer;
        use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

        let s = Store::open_memory().unwrap();
        let pi = "pi-rg";
        let proj = "reply-proj";

        // Use a fixed 64-char hex as the "root event id" (E1).
        // In production this comes from the published Nostr event id.
        let e1_hex = "a".repeat(64);

        // Seed the outbound root: project origin → thread origin keyed on E1.
        let pid = s
            .ensure_project_origin("kind1-nip29", pi, proj, proj, 100)
            .unwrap();
        let root_tid = s
            .ensure_thread_origin(&pid, "kind1-nip29", pi, &e1_hex, 100)
            .unwrap();
        let _root_mid = s
            .record_message(
                &root_tid,
                "pk-sender",
                "root message",
                100,
                "outbound",
                "published",
                Some(&e1_hex),
            )
            .unwrap();

        // Build a signed inbound reply event carrying ["e", E1, "", "root"].
        let recipient_keys = Keys::generate();
        let recipient_pk = recipient_keys.public_key().to_hex();

        // Create a recipient session so the legacy inbox path has somewhere to deliver.
        let rec = SessionRecord {
            session_id: "sess-reply-rg".into(),
            agent_slug: "agent".into(),
            agent_pubkey: recipient_pk.clone(),
            project: proj.into(),
            host: "host".into(),
            child_pid: None,
            watch_pid: None,
            created_at: 1,
            alive: true,
            rel_cwd: String::new(),
        };
        s.upsert_session(&rec).unwrap();

        let sender_keys = Keys::generate();
        // Build a reply event with a NIP-10 root e-tag pointing at E1.
        let reply_event = EventBuilder::new(Kind::from(1u16), "reply body")
            .tags([
                Tag::parse(["h", proj]).unwrap(),
                Tag::parse(["p", &recipient_pk]).unwrap(),
                Tag::parse(["e", &e1_hex, "", "root"]).unwrap(),
            ])
            .sign_with_keys(&sender_keys)
            .unwrap();

        let mention = Mention {
            from: AgentRef::new(sender_keys.public_key().to_hex(), "sender".to_string()),
            to_pubkey: recipient_pk.clone(),
            project: proj.into(),
            body: "reply body".into(),
            target_session: Some("sess-reply-rg".into()),
            from_session: None,
            meta: crate::domain::MentionMeta::default(),
        };

        Kind1Materializer::materialize_inbound_message(
            &s,
            &recipient_pk,
            &mention,
            &reply_event,
            pi,
            200,
        );

        // The reply must land in the SAME thread as the root.
        let thread_count: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE project_id=?1",
                params![pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            thread_count, 1,
            "reply must join the existing thread, not create a new one"
        );

        let msgs = s.messages_for_thread(&root_tid).unwrap();
        assert_eq!(msgs.len(), 2, "one root + one reply in the same thread");
        assert_eq!(msgs[0].direction, "outbound");
        assert_eq!(msgs[1].direction, "inbound");
        assert_eq!(msgs[1].body, "reply body");
    }

    /// Proposal dual-write: record_message + thread origin produce a canonical
    /// proposal row; idempotent on the kind:30023 event id.
    #[test]
    fn propose_dual_write_produces_canonical_row() {
        let s = Store::open_memory().unwrap();
        let pi = "test-pi-prop";
        let now = 3_000u64;
        let agent_pk = "cc".repeat(32);
        let owner_pk = "dd".repeat(32);
        let event_id = "abcd1234".repeat(8); // 64-char hex

        // Simulate rpc_propose's dual-write path.
        let project_id = s
            .ensure_project_origin("kind1-nip29", pi, "workspace", "workspace", now)
            .unwrap();
        // New standalone thread rooted at the proposal's event id.
        let thread_id = s
            .ensure_thread_origin(&project_id, "kind1-nip29", pi, &event_id, now)
            .unwrap();
        let msg_id = s
            .record_message(
                &thread_id,
                &agent_pk,
                "My Big Proposal", // title is the body in the canonical row
                now,
                "outbound",
                "published",
                Some(&event_id),
            )
            .unwrap();
        s.add_message_recipient(&msg_id, &owner_pk, None).unwrap();

        // Verify the canonical row.
        let msgs = s.messages_for_thread(&thread_id).unwrap();
        assert_eq!(msgs.len(), 1, "one message row for the proposal");
        let row = &msgs[0];
        assert_eq!(row.direction, "outbound");
        assert_eq!(row.sync_state, "published");
        assert_eq!(row.body, "My Big Proposal");
        assert_eq!(row.native_event_id.as_deref(), Some(event_id.as_str()));
        assert_eq!(row.author_pubkey, agent_pk);

        // Idempotency: the same event_id must not create a second message row.
        let mid2 = s
            .record_message(
                &thread_id,
                &agent_pk,
                "My Big Proposal (echo)",
                now,
                "outbound",
                "published",
                Some(&event_id),
            )
            .unwrap();
        assert_eq!(
            msg_id, mid2,
            "same native_event_id → same message_id (dedup)"
        );
        let msgs2 = s.messages_for_thread(&thread_id).unwrap();
        assert_eq!(
            msgs2.len(),
            1,
            "still one message row after idempotent write"
        );
    }

    /// Proposal attached to an existing thread: when --thread is given rpc_propose
    /// uses the thread_id directly as the target thread; the proposal message lands
    /// in that thread without creating a new one.
    #[test]
    fn propose_into_existing_thread() {
        let s = Store::open_memory().unwrap();
        let pi = "test-pi-prop2";
        let now = 4_000u64;
        let agent_pk = "ee".repeat(32);
        let thread_root_event_id = "1111aaaa".repeat(8);
        let proposal_event_id = "2222bbbb".repeat(8);

        // Pre-existing thread (e.g. created by a send-message earlier).
        let project_id = s
            .ensure_project_origin("kind1-nip29", pi, "workspace2", "workspace2", now)
            .unwrap();
        let existing_thread_id = s
            .ensure_thread_origin(&project_id, "kind1-nip29", pi, &thread_root_event_id, now)
            .unwrap();

        // rpc_propose with --thread: use the thread_id directly (no new ensure_thread_origin).
        // Mirror the dual-write code path in rpc_propose.
        let thread_id_for_proposal = existing_thread_id.clone();
        let msg_id = s
            .record_message(
                &thread_id_for_proposal,
                &agent_pk,
                "Proposal Title",
                now,
                "outbound",
                "published",
                Some(&proposal_event_id),
            )
            .unwrap();
        s.add_message_recipient(&msg_id, &agent_pk, None).unwrap();

        // Only one thread for this project.
        let threads = s.list_threads(&project_id).unwrap();
        assert_eq!(threads.len(), 1, "proposal must join existing thread");

        let msgs = s.messages_for_thread(&existing_thread_id).unwrap();
        assert_eq!(msgs.len(), 1, "one proposal message in the thread");
        assert_eq!(msgs[0].body, "Proposal Title");
        assert_eq!(
            msgs[0].native_event_id.as_deref(),
            Some(proposal_event_id.as_str())
        );
    }
}
