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
-- Current status scoped to one concrete session. This avoids showing one
-- Claude/Codex turn beside every sibling session that shares the same agent
-- pubkey in a project. `agent_status` remains as a legacy fallback for older
-- peers that publish agent-level status without a session-id tag.
CREATE TABLE IF NOT EXISTS session_status (
    pubkey     TEXT NOT NULL,
    project    TEXT NOT NULL,
    session_id TEXT NOT NULL,
    text       TEXT NOT NULL,
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
}

mod inbox;
mod peers;
mod projects;
mod sessions;

#[cfg(test)]
mod tests;

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
