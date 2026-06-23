//! Local app state in SQLite (M1 §2, §7).
//!
//! NMP-shaped event stores aside, tenex-edge keeps the *app* state the fabric
//! shouldn't own: my own sessions (+ the CC pid to watch), a directory of peers
//! built from their profiles/presence, and per-session chat inbox rows.

use crate::domain::Lifecycle;
use crate::session::{
    derive_status, DeltaKind, IdentityDecision, LiveLocator, PeerStatusObservation,
    SessionObservation, SessionSnapshot, SnapshotSource, StatusDeltaItem, TitleSource,
};
use crate::util::SessionId;
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
    /// User-chosen NIP-29 subgroup h-tag the session is operating within
    /// (distinct from `project`, which is the per-session room h-tag).
    pub channel: String,
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
pub struct ChatInboxRow {
    pub chat_event_id: String,
    pub target_session: String,
    pub from_pubkey: String,
    pub from_slug: String,
    pub project: String,
    pub body: String,
    pub created_at: u64,
    pub from_session: String,
    pub mentioned_session: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLogRow {
    pub chat_event_id: String,
    pub from_pubkey: String,
    pub from_slug: String,
    pub host: String,
    pub project: String,
    pub body: String,
    pub created_at: u64,
    pub from_session: String,
    pub mentioned_session: String,
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

/// One pending kind:30315 publication, returned by `pending_status_outbox`.
/// `snapshot` is the CURRENT `session_state` row for `session_id` (the drainer
/// publishes the latest fact; older pending versions coalesce). The drainer
/// builds a `Status` from `snapshot`, sets `expires_at = now + STATUS_TTL_SECS`,
/// calls `Kind1Nip29Provider::set_status`, then `mark_status_published`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusOutboxItem {
    pub session_id: String,
    pub state_version: i64,
    pub retries: i64,
    pub snapshot: SessionSnapshot,
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
    rel_cwd       TEXT NOT NULL DEFAULT '',
    channel       TEXT NOT NULL DEFAULT ''
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
CREATE TABLE IF NOT EXISTS chat_inbox (
    chat_event_id     TEXT NOT NULL,
    target_session    TEXT NOT NULL,
    from_pubkey       TEXT NOT NULL,
    from_slug         TEXT NOT NULL,
    project           TEXT NOT NULL,
    body              TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    delivered         INTEGER NOT NULL DEFAULT 0,
    delivered_at      INTEGER NOT NULL DEFAULT 0,
    from_session      TEXT NOT NULL DEFAULT '',
    mentioned_session TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (chat_event_id, target_session)
);
CREATE TABLE IF NOT EXISTS chat_messages (
    chat_event_id     TEXT PRIMARY KEY,
    from_pubkey       TEXT NOT NULL,
    from_slug         TEXT NOT NULL,
    host              TEXT NOT NULL DEFAULT '',
    project           TEXT NOT NULL,
    body              TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    from_session      TEXT NOT NULL DEFAULT '',
    mentioned_session TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_chat_messages_project_created
    ON chat_messages(project, created_at, chat_event_id);
-- Per-session turn state: flipped by the host's turn-start/turn-end hooks. The
-- engine polls this to decide when to distill activity (30s into a turn, then
-- every few minutes) and when to go idle. No tool events — distillation reads
-- the conversation transcript, not tool names.
CREATE TABLE IF NOT EXISTS turn_state (
    session_id      TEXT PRIMARY KEY,
    working         INTEGER NOT NULL DEFAULT 0,
    turn_started_at INTEGER NOT NULL DEFAULT 0,
    -- Mid-turn delta cursor: timestamp of the last PostToolUse turn_check.
    -- Reset to 0 at turn start so each in-turn check reports only sibling
    -- changes since the previous check (the guarded ALTER below migrates
    -- pre-existing on-disk databases that predate this column).
    last_check_at   INTEGER NOT NULL DEFAULT 0
);
-- ── canonical session aggregate (single source of truth) ─────────────────────
-- ONE row per local session keyed by the daemon-minted canonical session_id.
-- Holds the whole public fact (title/activity/busy/phase/turn/lifecycle) plus
-- the liveness clock (last_seen) and the delta cursors (first_seen set ONLY on
-- insert, updated_at bumped in lockstep with state_version on every public
-- content change — NEVER on a bare heartbeat). All mutation flows through the
-- Store transition methods, each one txn that bumps state_version and enqueues a
-- status_outbox row when public status changed.
CREATE TABLE IF NOT EXISTS session_state (
    session_id      TEXT PRIMARY KEY,
    agent_slug      TEXT NOT NULL,
    agent_pubkey    TEXT NOT NULL,
    project         TEXT NOT NULL,
    host            TEXT NOT NULL,
    rel_cwd         TEXT NOT NULL DEFAULT '',
    title           TEXT NOT NULL DEFAULT '',
    title_source    TEXT NOT NULL DEFAULT 'none',
    activity        TEXT NOT NULL DEFAULT '',
    busy            INTEGER NOT NULL DEFAULT 0,
    phase           TEXT NOT NULL DEFAULT 'idle',
    turn_id         INTEGER NOT NULL DEFAULT 0,
    turn_started_at INTEGER NOT NULL DEFAULT 0,
    last_distill_at INTEGER NOT NULL DEFAULT 0,
    last_seen       INTEGER NOT NULL DEFAULT 0,
    resume_id       TEXT NOT NULL DEFAULT '',
    state_version   INTEGER NOT NULL DEFAULT 0,
    lifecycle       TEXT NOT NULL DEFAULT 'active',
    first_seen      INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_session_state_project_seen
    ON session_state(project, last_seen);
CREATE INDEX IF NOT EXISTS idx_session_state_project_updated
    ON session_state(project, updated_at);
-- Maps every external identifier (harness-native id, resume token, tmux pane,
-- watch pid, generated te-* id) to the canonical session_id. (harness,
-- external_id_kind, external_id) is the PK so the same raw id under two harnesses
-- or two kinds never collide.
CREATE TABLE IF NOT EXISTS session_aliases (
    harness          TEXT NOT NULL,
    external_id_kind TEXT NOT NULL,
    external_id      TEXT NOT NULL,
    session_id       TEXT NOT NULL,
    created_at       INTEGER NOT NULL,
    PRIMARY KEY (harness, external_id_kind, external_id)
);
CREATE INDEX IF NOT EXISTS idx_session_aliases_session
    ON session_aliases(session_id);
-- Peer mirror, materialized from inbound kind:30315. Keyed by (pubkey, project):
-- one row per agent per group. `project` == the kind:30315 `d` tag == `h` tag ==
-- project slug. A newer heartbeat replaces the older row for the same agent+group.
-- Same delta cursors as session_state so status_delta_since works across both.
-- last_seen = the event's emitted-at (a finished peer stops emitting → ages out);
-- never local-writable. No native_session_id — issue #5 §4.
CREATE TABLE IF NOT EXISTS peer_session_state (
    pubkey            TEXT NOT NULL,
    project           TEXT NOT NULL,
    agent_slug        TEXT NOT NULL DEFAULT '',
    host              TEXT NOT NULL DEFAULT '',
    rel_cwd           TEXT NOT NULL DEFAULT '',
    title             TEXT NOT NULL DEFAULT '',
    activity          TEXT NOT NULL DEFAULT '',
    busy              INTEGER NOT NULL DEFAULT 0,
    last_seen         INTEGER NOT NULL DEFAULT 0,
    state_version     INTEGER NOT NULL DEFAULT 0,
    lifecycle         TEXT NOT NULL DEFAULT 'active',
    first_seen        INTEGER NOT NULL DEFAULT 0,
    updated_at        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (pubkey, project)
);
CREATE INDEX IF NOT EXISTS idx_peer_session_state_project_seen
    ON peer_session_state(project, last_seen);
-- Desired kind:30315 publications. One row per (session_id, state_version): the
-- daemon drainer publishes it via Kind1Nip29Provider::set_status, records the
-- native event id, and retries on failure. Only versioned CONTENT changes land
-- here; the per-heartbeat liveness re-arm republishes the latest snapshot WITHOUT
-- an outbox row.
CREATE TABLE IF NOT EXISTS status_outbox (
    session_id      TEXT NOT NULL,
    state_version   INTEGER NOT NULL,
    publish_state   TEXT NOT NULL DEFAULT 'pending',
    native_event_id TEXT,
    retries         INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    enqueued_at     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, state_version)
);
CREATE INDEX IF NOT EXISTS idx_status_outbox_pending
    ON status_outbox(publish_state, enqueued_at);
-- NIP-29 group metadata cache: the 'about' text for each project channel (kind 39000).
CREATE TABLE IF NOT EXISTS project_meta (
    project    TEXT PRIMARY KEY,
    about      TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    -- NIP-29 subgroup hierarchy (issue #3): `name` is the human display name from
    -- the relay-authored kind:39000 `name` tag; `parent` is the parent group id
    -- from its `parent` tag (empty for top-level project groups). Lets
    -- `groups list` render the tree from local state without hitting the relay.
    name       TEXT NOT NULL DEFAULT '',
    parent     TEXT NOT NULL DEFAULT ''
);
-- NIP-29 groups this daemon owns/manages (created + locked closed via tenexPrivateKey).
CREATE TABLE IF NOT EXISTS owned_groups (
    project    TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    -- 1 when this owned group is a per-session room (issue #6), so only the
    -- owning session auto-renames it to its distilled title. The ALTER in `open`
    -- backfills this column for databases created before the column existed.
    is_session_room INTEGER NOT NULL DEFAULT 0,
    -- The work-root project a per-session room is nested under. Set at mint and
    -- NOT touched by the relay materializer (unlike project_meta.parent, which a
    -- relay that doesn't re-emit the NIP-29 parent tag can clobber). Lets
    -- host-side resolution find a session by its work-root now that the room id
    -- (session-<hash>) no longer encodes the project name.
    room_parent TEXT NOT NULL DEFAULT ''
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
-- Durable dedup for subgroup add-agents orchestration events (issue #3). The
-- relay redelivers the same kind:9 on every matching subscription, and a daemon
-- restart replays history; this table makes provisioning fire AT MOST ONCE per
-- event id, surviving restarts (unlike the in-memory first_sight cache).
CREATE TABLE IF NOT EXISTS processed_orchestration (
    event_id     TEXT PRIMARY KEY,
    processed_at INTEGER NOT NULL
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
-- Per-session distillation error log. Written by the runtime when the LLM
-- distiller fails; read by rpc_statusline to flash a warning. One row per
-- session (upsert) so only the last error is kept — the log file has full history.
CREATE TABLE IF NOT EXISTS session_errors (
    session_id TEXT PRIMARY KEY,
    message    TEXT NOT NULL,
    ts         INTEGER NOT NULL
);
-- TMUX control-plane: one row per (session, kind='tmux') endpoint. Written by
-- rpc_session_start when the hook env supplies TMUX_PANE; read by the pending
-- message dispatcher. `target` is the stable tmux pane id (e.g. '%5'). `meta` is a JSON
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
-- Stage 3 (Issue #2): derived per-session Nostr pubkeys. Maps the pubkey that
-- results from `identity::derive_session_keys` back to the owning session.
-- Populated on session_start; cleared on session_end / engine self-exit /
-- crash-GC. Used by two subsystems:
--   1. Routing: a mention p-tagged to a session pubkey resolves to the owning
--      session via `session_pubkey_info` in `route_mention_into_with_id`.
--   2. Slug resolution: `resolve_slug_for_pubkey(session_pubkey)` fabricates
--      "<codename> (<agent_slug>)" from this table so inbound session-signed
--      events render a sensible sender name without a round-trip to the relay.
CREATE TABLE IF NOT EXISTS session_pubkeys (
    session_pubkey  TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    agent_pubkey    TEXT NOT NULL,
    agent_slug      TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_pubkeys_session
    ON session_pubkeys(session_id);
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
        // NIP-29 subgroup hierarchy columns on project_meta (issue #3).
        let _ = conn.execute(
            "ALTER TABLE project_meta ADD COLUMN name TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE project_meta ADD COLUMN parent TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN last_seen INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Issue #6: mark owned groups that are per-session rooms (vs project /
        // task groups), so only the owning session auto-renames them to its
        // distilled title.
        let _ = conn.execute(
            "ALTER TABLE owned_groups ADD COLUMN is_session_room INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE owned_groups ADD COLUMN room_parent TEXT NOT NULL DEFAULT ''",
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
            "ALTER TABLE sessions ADD COLUMN channel TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE peer_sessions ADD COLUMN rel_cwd TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Snapshot of the last assistant text at the beginning of each turn.
        // Used by rpc_turn_end to poll until a *new* response appears in the
        // transcript (Claude Code writes the transcript after the stop hook fires).
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN last_assistant_text_at_turn_start TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Session-state rearchitecture: the legacy `agent_status` / `session_status`
        // tables are replaced wholesale by the canonical `session_state` +
        // `peer_session_state` aggregate. No backwards compatibility — drop them so
        // a stale schema can't be read by accident.
        let _ = conn.execute("DROP TABLE IF EXISTS agent_status", []);
        let _ = conn.execute("DROP TABLE IF EXISTS session_status", []);
        // Issue #5 §4: peer_session_state PK changed from (pubkey, project,
        // native_session_id) to (pubkey, project). No backwards compat — drop and
        // recreate when the old native_session_id column is still present.
        {
            let has_old: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('peer_session_state') WHERE name='native_session_id'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            if has_old {
                conn.execute_batch(
                    "DROP TABLE IF EXISTS peer_session_state;
                     DROP INDEX IF EXISTS idx_peer_session_state_project_seen;
                     CREATE TABLE IF NOT EXISTS peer_session_state (
                         pubkey            TEXT NOT NULL,
                         project           TEXT NOT NULL,
                         agent_slug        TEXT NOT NULL DEFAULT '',
                         host              TEXT NOT NULL DEFAULT '',
                         rel_cwd           TEXT NOT NULL DEFAULT '',
                         title             TEXT NOT NULL DEFAULT '',
                         activity          TEXT NOT NULL DEFAULT '',
                         busy              INTEGER NOT NULL DEFAULT 0,
                         last_seen         INTEGER NOT NULL DEFAULT 0,
                         state_version     INTEGER NOT NULL DEFAULT 0,
                         lifecycle         TEXT NOT NULL DEFAULT 'active',
                         first_seen        INTEGER NOT NULL DEFAULT 0,
                         updated_at        INTEGER NOT NULL DEFAULT 0,
                         PRIMARY KEY (pubkey, project)
                     );
                     CREATE INDEX IF NOT EXISTS idx_peer_session_state_project_seen
                         ON peer_session_state(project, last_seen);",
                )
                .ok();
            }
        }
        // Harness-native resume token (e.g. the id `claude --resume <id>` /
        // `codex resume <id>` / `opencode --session <id>` wants). For claude-code
        // and codex this equals `session_id` (they assign their own id, which we
        // adopt); for opencode it is the `ses_*` id the plugin forwards, distinct
        // from our synthetic `te-*` identity. Empty = not resumable.
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN resume_id TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Mid-turn delta cursor: timestamp of the last PostToolUse `turn_check`
        // for this session. Lets each in-turn check report only sibling-session
        // changes since the previous check (gated by deltas), instead of re-
        // emitting the whole roster on every tool call. Reset to 0 at turn start.
        let _ = conn.execute(
            "ALTER TABLE turn_state ADD COLUMN last_check_at INTEGER NOT NULL DEFAULT 0",
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
               (session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(session_id) DO UPDATE SET
               agent_slug=?2, agent_pubkey=?3, project=?4, host=?5,
               child_pid=?6, watch_pid=?7, alive=?9, rel_cwd=?10, channel=?11",
            params![
                r.session_id, r.agent_slug, r.agent_pubkey, r.project, r.host,
                r.child_pid, r.watch_pid, r.created_at, r.alive as i32, r.rel_cwd, r.channel
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> Result<Option<SessionRecord>> {
        if let Some(rec) = self.get_session_exact(id)? {
            return Ok(Some(rec));
        }
        // Fallback: `id` may be a harness external id (claude/codex native id,
        // opencode resume token, tmux pane, watch pid). Resolve it to the canonical
        // session via `session_aliases`, newest mapping wins.
        let canonical: Option<String> = self
            .conn
            .query_row(
                "SELECT session_id FROM session_aliases
                 WHERE external_id=?1 ORDER BY created_at DESC LIMIT 1",
                params![id],
                |r| r.get::<_, String>(0),
            )
            .ok();
        match canonical {
            Some(canon) if canon != id => self.get_session_exact(&canon),
            _ => Ok(None),
        }
    }

    /// Resolve a possibly-aliased harness/external session id (or an already-
    /// canonical id) to the canonical `session_state` id. Hooks speak harness
    /// ids; every canonical transition (start_turn/end_turn/end_session/…) must
    /// be keyed by the minted canonical id or it silently updates zero rows.
    /// Returns the input unchanged when it is already canonical or has no alias
    /// mapping (so a brand-new id still flows through to registration).
    pub fn canonical_session_id(&self, id: &str) -> String {
        let is_canonical: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM session_state WHERE session_id=?1)",
                params![id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if is_canonical {
            return id.to_string();
        }
        self.conn
            .query_row(
                "SELECT session_id FROM session_aliases
                 WHERE external_id=?1 ORDER BY created_at DESC LIMIT 1",
                params![id],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .unwrap_or_else(|| id.to_string())
    }

    /// All locally-owned live sessions whose liveness is still fresh
    /// (`last_seen >= fresh_since`). Drives the daemon's heartbeat re-arm: every
    /// cadence these are re-published so the kind:30315 NIP-40 expiration is
    /// pushed forward and a live-but-idle session never ages off the relay.
    pub fn all_live_local_snapshots(&self, fresh_since: u64) -> Result<Vec<SessionSnapshot>> {
        let sql = format!(
            "SELECT {SESSION_STATE_COLS} FROM session_state
             WHERE lifecycle='active' AND last_seen>=?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![fresh_since], row_to_session_state)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Direct lookup of a session by its canonical id (no alias resolution).
    fn get_session_exact(&self, id: &str) -> Result<Option<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel
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
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel
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
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel
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
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel
             FROM sessions WHERE alive=1 AND project=?1 AND agent_slug=?2 ORDER BY created_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![project, agent_slug])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_session(row)?))
        } else {
            Ok(None)
        }
    }

    /// Most-recent still-alive session for an agent under a `work_root` — the
    /// bare project OR any per-session room minted beneath it
    /// (`<slug>-<hex8>`). A human-initiated session is stored under its minted
    /// room (issue #6), but the same terminal's later `tenex-edge` verbs only
    /// resolve the bare work-root from `cwd` (no `TENEX_EDGE_CHANNEL` is exported
    /// into an already-running interactive shell). Without this the agent can't
    /// find the very session it is running inside. Pass `None` for `agent` to
    /// match any agent in the work-root.
    pub fn latest_alive_session_under_work_root(
        &self,
        work_root: &str,
        agent_slug: Option<&str>,
    ) -> Result<Option<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel
             FROM sessions WHERE alive=1 AND (?2 IS NULL OR agent_slug=?2) ORDER BY created_at DESC",
        )?;
        let mut rows = stmt.query(params![work_root, agent_slug])?;
        while let Some(row) = rows.next()? {
            let rec = row_to_session(row)?;
            // The session is either directly in the bare work-root project, or in
            // a per-session room nested under it. The room id (session-<hash>) no
            // longer encodes the project, so match on the stored room_parent.
            let under_work_root = rec.project == work_root
                || self.session_room_parent(&rec.project)?.as_deref() == Some(work_root);
            if under_work_root {
                return Ok(Some(rec));
            }
        }
        Ok(None)
    }

    /// Persist the harness-native resume token for a session. Idempotent; a
    /// later call with the same token is a no-op. Never clears a known token with
    /// an empty one (so a stray payload can't wipe a good resume id).
    pub fn set_session_resume_id(&self, session_id: &str, resume_id: &str) -> Result<()> {
        if resume_id.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "UPDATE sessions SET resume_id=?2 WHERE session_id=?1",
            params![session_id, resume_id],
        )?;
        Ok(())
    }

    /// The harness-native resume token for a session, or `None` when unset/empty.
    pub fn get_session_resume_id(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT resume_id FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .filter(|s| !s.is_empty()))
    }

    /// Recent sessions on `host`, newest first, with their stored `resume_id`
    /// (which may be empty — claude/codex sessions use their `session_id` as the
    /// resume token, so the caller derives the real token rather than filtering
    /// here). Includes dead (`alive=0`) rows. Returns `(record, resume_id)` pairs.
    pub fn list_resumable_sessions(
        &self,
        host: &str,
        limit: usize,
    ) -> Result<Vec<(SessionRecord, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel, resume_id
             FROM sessions WHERE host=?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![host, limit as i64], |row| {
            let rec = row_to_session(row)?;
            let resume_id: String = row.get(11)?;
            Ok((rec, resume_id))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
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
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel
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

    /// Canonical `agent@host` lookup: resolve a durable agent on a specific
    /// machine — `(slug, slugify_host(host))` → pubkey. Scans kind:0 profiles
    /// (covers remote agents) then own sessions, filtering by the slugified host.
    /// This is the one place `@host` addressing resolves; `@` never means project.
    pub fn pubkey_for_agent_on_host(&self, slug: &str, host_slug: &str) -> Result<Option<String>> {
        let scan = |sql: &str| -> Option<String> {
            let mut stmt = self.conn.prepare(sql).ok()?;
            let rows = stmt
                .query_map(params![slug], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .ok()?;
            for (pubkey, host) in rows.flatten() {
                if crate::util::slugify_host(&host) == host_slug {
                    return Some(pubkey);
                }
            }
            None
        };
        if let Some(pk) =
            scan("SELECT pubkey, host FROM profiles WHERE slug=?1 ORDER BY updated_at DESC")
        {
            return Ok(Some(pk));
        }
        if let Some(pk) = scan(
            "SELECT agent_pubkey, host FROM sessions WHERE agent_slug=?1 ORDER BY created_at DESC",
        ) {
            return Ok(Some(pk));
        }
        Ok(None)
    }

    /// Resolve a backend pubkey from a host slug (the `slugify_host` form shown
    /// by `who`). Queries peer_sessions then profiles for any row whose host,
    /// when slugified, equals `host_slug`. Returns the most-recently-seen pubkey,
    /// or `None` if no peer is known under that host name.
    pub fn pubkey_for_host_slug(&self, host_slug: &str) -> Option<String> {
        use crate::util::slugify_host;
        // peer_sessions: prefer the most-recently active backend on that host.
        {
            let mut stmt = self
                .conn
                .prepare("SELECT pubkey, host FROM peer_sessions ORDER BY last_seen DESC")
                .ok()?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .ok()?;
            for row in rows.flatten() {
                if slugify_host(&row.1) == host_slug {
                    return Some(row.0);
                }
            }
        }
        // profiles: fall back to kind:0 identity cards.
        {
            let mut stmt = self
                .conn
                .prepare("SELECT pubkey, host FROM profiles ORDER BY updated_at DESC")
                .ok()?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .ok()?;
            for row in rows.flatten() {
                if slugify_host(&row.1) == host_slug {
                    return Some(row.0);
                }
            }
        }
        None
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
        // Fall back to profiles table (populated by kind:0 events from peers).
        if let Ok(slug) = self.conn.query_row(
            "SELECT slug FROM profiles WHERE pubkey=?1 LIMIT 1",
            params![pubkey],
            |r| r.get::<_, String>(0),
        ) {
            return Ok(Some(slug));
        }
        // Stage 3: check if the pubkey is a per-session derived key. Local
        // sessions skip profile materialization (is_self gate), so the profiles
        // table won't have an entry. Fabricate "<codename> (<agent_slug>)"
        // matching the session kind:0 we publish with the session key.
        if let Some((session_id, _agent_pubkey, agent_slug)) = self.session_pubkey_info(pubkey) {
            let codename = crate::util::session_codename(&session_id);
            return Ok(Some(format!("{codename} ({agent_slug})")));
        }
        Ok(None)
    }

    pub fn resolve_chat_host(
        &self,
        pubkey: &str,
        from_session: Option<&str>,
    ) -> Result<Option<String>> {
        if let Some(session_id) = from_session.filter(|s| !s.is_empty()) {
            if let Ok(host) = self.conn.query_row(
                "SELECT host FROM sessions WHERE session_id=?1 LIMIT 1",
                params![session_id],
                |r| r.get::<_, String>(0),
            ) {
                return Ok(Some(host));
            }
            if let Ok(host) = self.conn.query_row(
                "SELECT host FROM peer_sessions WHERE session_id=?1 LIMIT 1",
                params![session_id],
                |r| r.get::<_, String>(0),
            ) {
                return Ok(Some(host));
            }
        }
        if let Ok(host) = self.conn.query_row(
            "SELECT host FROM sessions WHERE agent_pubkey=?1 ORDER BY created_at DESC LIMIT 1",
            params![pubkey],
            |r| r.get::<_, String>(0),
        ) {
            return Ok(Some(host));
        }
        if let Ok(host) = self.conn.query_row(
            "SELECT host FROM peer_sessions WHERE pubkey=?1 ORDER BY last_seen DESC LIMIT 1",
            params![pubkey],
            |r| r.get::<_, String>(0),
        ) {
            return Ok(Some(host));
        }
        Ok(self
            .conn
            .query_row(
                "SELECT host FROM profiles WHERE pubkey=?1 LIMIT 1",
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
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel
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

    // ── turn state (drives distillation) ─────────────────────────────────

    /// Mark a session as actively working on a turn, stamping its start time.
    /// Idempotent within a turn; a fresh `ts` signals a new turn to the engine.
    pub fn mark_turn_start(&self, session_id: &str, ts: u64) -> Result<()> {
        // Reset the mid-turn delta cursor (last_check_at=0) so the first
        // PostToolUse of the new turn reports sibling changes since `ts`.
        self.conn.execute(
            "INSERT INTO turn_state (session_id, working, turn_started_at, last_check_at) VALUES (?1, 1, ?2, 0)
             ON CONFLICT(session_id) DO UPDATE SET working=1, turn_started_at=?2, last_check_at=0",
            params![session_id, ts],
        )?;
        Ok(())
    }

    /// Mark a session idle (the turn ended). The engine publishes idle status on
    /// its next poll.
    pub fn mark_turn_end(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO turn_state (session_id, working, turn_started_at, last_check_at) VALUES (?1, 0, 0, 0)
             ON CONFLICT(session_id) DO UPDATE SET working=0, last_check_at=0",
            params![session_id],
        )?;
        Ok(())
    }

    /// Mid-turn delta gate for PostToolUse `turn_check`. If the session is in a
    /// turn AND at least `min_interval` seconds have passed since the last check
    /// (or since turn start, if no check yet this turn), advance the cursor to
    /// `now` and return `Some(since)` — the timestamp to query sibling-session
    /// deltas from. Returns `None` when not in a turn or rate-limited, so the
    /// hook stays silent. The write is safe here: `turn_check` is daemon-
    /// mediated, so the daemon's single store connection is the only writer.
    pub fn turn_check_due(
        &self,
        session_id: &str,
        now: u64,
        min_interval: u64,
    ) -> Result<Option<u64>> {
        let (working, turn_started_at, last_check_at) = self
            .conn
            .query_row(
                "SELECT working, turn_started_at, last_check_at FROM turn_state WHERE session_id=?1",
                params![session_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)? != 0,
                        r.get::<_, i64>(1)? as u64,
                        r.get::<_, i64>(2)? as u64,
                    ))
                },
            )
            .unwrap_or((false, 0, 0));
        // Only mid-turn (turn_end leaves turn_started_at set but clears working).
        // The turn_started_at guard also avoids querying all history pre-turn.
        if !working || turn_started_at == 0 {
            return Ok(None);
        }
        let since = if last_check_at > 0 {
            // Subsequent check this turn: enforce the floor against the last one.
            if now.saturating_sub(last_check_at) < min_interval {
                return Ok(None);
            }
            last_check_at
        } else {
            // First check of the turn → always due; window opens at turn start.
            turn_started_at
        };
        self.conn.execute(
            "UPDATE turn_state SET last_check_at=?2 WHERE session_id=?1",
            params![session_id, now],
        )?;
        Ok(Some(since))
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
    /// Defaults to `false` (not working) when no row exists, so tmux injection is
    /// allowed when a session has never started a turn.
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

    /// Count undelivered chat rows that explicitly mention this session.
    pub fn count_unread_chat_mentions(&self, session_id: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chat_inbox
             WHERE target_session=?1 AND mentioned_session=?1 AND delivered=0",
            params![session_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    // ── canonical session aggregate: identity registry ───────────────────────

    /// Register a freshly-observed session, returning the canonical snapshot.
    ///
    /// One SQLite transaction: resolve identity via the pure `resolve_identity`
    /// helper (alias hit → reassert; live pane/pid slot reused → supersede old +
    /// mint; else mint), write/refresh `session_aliases`, bump `state_version`,
    /// and enqueue a `status_outbox` row so the daemon publishes the session's
    /// kind:30315. The hook reports a normalized `SessionObservation`; THIS owns
    /// identity policy.
    pub fn register_or_reassert_session(
        &self,
        obs: &SessionObservation,
    ) -> Result<SessionSnapshot> {
        let alias_hit = self.alias_lookup(obs);
        let live = self.live_locators_for(&obs.host, &obs.project, &obs.agent_pubkey, obs)?;
        let decision = crate::session::resolve_identity(obs, alias_hit, &live);
        let id = match decision {
            IdentityDecision::Existing(id) | IdentityDecision::Reattach(id) => {
                self.reassert_session_row(id.as_str(), obs)?;
                id.into_string()
            }
            IdentityDecision::Supersede { old } => {
                self.supersede_session(old.as_str(), obs.observed_at)?;
                let id = mint_session_id();
                self.insert_session_row(&id, obs)?;
                id
            }
            IdentityDecision::Mint => {
                let id = mint_session_id();
                self.insert_session_row(&id, obs)?;
                id
            }
        };
        self.write_session_aliases(&id, obs)?;
        Ok(self
            .local_session_snapshot(&id)?
            .expect("session_state row written by register_or_reassert_session"))
    }

    /// Existing-id path: refresh mutable identity fields + liveness. Only bump
    /// the version / `updated_at` when the public status actually changed.
    fn reassert_session_row(&self, session_id: &str, obs: &SessionObservation) -> Result<()> {
        let before = self.local_session_snapshot(session_id)?;
        let public_changed = before
            .as_ref()
            .map(|s| {
                s.agent_slug != obs.agent_slug
                    || s.host != obs.host
                    || s.rel_cwd != obs.rel_cwd
                    || !s.lifecycle.is_active()
            })
            .unwrap_or(true);

        if public_changed {
            self.conn.execute(
                "UPDATE session_state SET
                   agent_slug=?2, host=?3, rel_cwd=?4,
                   resume_id=CASE WHEN ?5<>'' THEN ?5 ELSE resume_id END,
                   last_seen=?6, lifecycle='active',
                   state_version=state_version+1, updated_at=?6
                 WHERE session_id=?1",
                params![
                    session_id,
                    obs.agent_slug,
                    obs.host,
                    obs.rel_cwd,
                    obs.resume_id.clone().unwrap_or_default(),
                    obs.observed_at,
                ],
            )?;
            self.enqueue_status_outbox_current(session_id, obs.observed_at)
        } else {
            self.conn.execute(
                "UPDATE session_state SET
                   resume_id=CASE WHEN ?2<>'' THEN ?2 ELSE resume_id END,
                   last_seen=?3, lifecycle='active'
                 WHERE session_id=?1",
                params![
                    session_id,
                    obs.resume_id.clone().unwrap_or_default(),
                    obs.observed_at,
                ],
            )?;
            Ok(())
        }
    }

    /// Mint-path insert: a brand-new canonical row at version 1.
    fn insert_session_row(&self, session_id: &str, obs: &SessionObservation) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_state
               (session_id, agent_slug, agent_pubkey, project, host, rel_cwd,
                title, title_source, activity, busy, phase, turn_id, turn_started_at,
                last_distill_at, last_seen, resume_id, state_version, lifecycle,
                first_seen, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6, '', 'none', '', 0, 'idle', 0, 0,
                     0, ?7, ?8, 1, 'active', ?7, ?7)",
            params![
                session_id,
                obs.agent_slug,
                obs.agent_pubkey,
                obs.project,
                obs.host,
                obs.rel_cwd,
                obs.observed_at,
                obs.resume_id.clone().unwrap_or_default(),
            ],
        )?;
        // Keep `sessions` in sync: rpc_session_start also calls upsert_session, but
        // the reassert path (user-prompt-submit → register_or_reassert_session) does
        // not, leaving get_session_exact unable to find the canonical row. Also set
        // last_seen so list_my_live_sessions (last_seen>=?) finds the row before the
        // rpc_session_start touch_session arrives.
        self.conn.execute(
            "INSERT INTO sessions
               (session_id, agent_slug, agent_pubkey, project, host,
                child_pid, watch_pid, created_at, alive, rel_cwd, last_seen)
             VALUES (?1,?2,?3,?4,?5, NULL,?6,?7,1,?8,?7)
             ON CONFLICT(session_id) DO NOTHING",
            params![
                session_id,
                obs.agent_slug,
                obs.agent_pubkey,
                obs.project,
                obs.host,
                obs.watch_pid,
                obs.observed_at,
                obs.rel_cwd,
            ],
        )?;
        self.enqueue_status_outbox(session_id, 1, obs.observed_at)
    }

    /// Upsert every external id the observation carries → this canonical id.
    /// pane/pid/harness aliases are repointed to the newest session so a reused
    /// slot resolves to the live owner.
    fn write_session_aliases(&self, session_id: &str, obs: &SessionObservation) -> Result<()> {
        use crate::session::AliasKind::*;
        let h = obs.harness.as_str();
        let put = |kind: &str, val: &str| -> Result<()> {
            if val.is_empty() {
                return Ok(());
            }
            self.conn.execute(
                "INSERT INTO session_aliases (harness, external_id_kind, external_id, session_id, created_at)
                 VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(harness, external_id_kind, external_id)
                 DO UPDATE SET session_id=?4, created_at=?5",
                params![h, kind, val, session_id, obs.observed_at],
            )?;
            Ok(())
        };
        if let Some(v) = &obs.harness_session_id {
            put(HarnessSession.as_str(), v)?;
        }
        if let Some(v) = &obs.resume_id {
            put(Resume.as_str(), v)?;
        }
        if let Some(v) = &obs.tmux_pane {
            put(TmuxPane.as_str(), v)?;
        }
        if let Some(pid) = obs.watch_pid {
            put(WatchPid.as_str(), &pid.to_string())?;
        }
        Ok(())
    }

    /// Alias hit (Existing) consults only harness-native id + resume kinds — a
    /// pane/pid alias from a prior occupant must NOT read as the same session.
    /// Returns the canonical id when one is found AND its row still exists.
    fn alias_lookup(&self, obs: &SessionObservation) -> Option<SessionId> {
        use crate::session::AliasKind;
        let h = obs.harness.as_str();
        // Echo harnesses (e.g. opencode) own no native id, so the daemon mints the
        // canonical id at session-start and echoes it back; the harness then reports
        // it as its own `harness_session_id` on every later hook. That id IS the
        // session — recognize it directly so a reassert REATTACHES instead of falling
        // through to the pane/pid supersede branch and minting a fresh session each
        // first turn. Safe for claude/codex: their native ids are never `te-*`
        // canonical ids, so this never matches for them.
        if let Some(v) = &obs.harness_session_id {
            if !v.is_empty() {
                let is_canonical: bool = self
                    .conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM session_state WHERE session_id=?1)",
                        params![v],
                        |r| r.get(0),
                    )
                    .unwrap_or(false);
                if is_canonical {
                    return Some(SessionId::from(v.clone()));
                }
            }
        }
        let mut candidates: Vec<(&str, &str)> = Vec::new();
        if let Some(v) = &obs.harness_session_id {
            candidates.push((AliasKind::HarnessSession.as_str(), v));
        }
        if let Some(v) = &obs.resume_id {
            candidates.push((AliasKind::Resume.as_str(), v));
        }
        for (kind, val) in candidates {
            if val.is_empty() {
                continue;
            }
            let found: Option<String> = self
                .conn
                .query_row(
                    "SELECT a.session_id FROM session_aliases a
                     JOIN session_state s ON s.session_id=a.session_id
                     WHERE a.harness=?1 AND a.external_id_kind=?2 AND a.external_id=?3",
                    params![h, kind, val],
                    |r| r.get::<_, String>(0),
                )
                .ok();
            if let Some(id) = found {
                return Some(SessionId::from(id));
            }
        }
        None
    }

    /// Live (active + fresh) session candidates on the same (host, project,
    /// agent), with their pane/pid/harness/resume locators joined from
    /// `session_aliases` — the input to `resolve_identity`'s supersede branch.
    fn live_locators_for(
        &self,
        host: &str,
        project: &str,
        agent_pubkey: &str,
        obs: &SessionObservation,
    ) -> Result<Vec<LiveLocator>> {
        use crate::session::AliasKind;
        let fresh_since = obs
            .observed_at
            .saturating_sub(crate::domain::STATUS_TTL_SECS);
        let h = obs.harness.as_str();
        let mut stmt = self.conn.prepare(
            "SELECT s.session_id,
               (SELECT external_id FROM session_aliases a WHERE a.session_id=s.session_id AND a.harness=?1 AND a.external_id_kind=?5),
               (SELECT external_id FROM session_aliases a WHERE a.session_id=s.session_id AND a.harness=?1 AND a.external_id_kind=?6),
               (SELECT external_id FROM session_aliases a WHERE a.session_id=s.session_id AND a.harness=?1 AND a.external_id_kind=?7),
               (SELECT external_id FROM session_aliases a WHERE a.session_id=s.session_id AND a.harness=?1 AND a.external_id_kind=?8)
             FROM session_state s
             WHERE s.lifecycle='active' AND s.host=?2 AND s.project=?3 AND s.agent_pubkey=?4
               AND s.last_seen>=?9",
        )?;
        let rows = stmt
            .query_map(
                params![
                    h,
                    host,
                    project,
                    agent_pubkey,
                    AliasKind::HarnessSession.as_str(),
                    AliasKind::Resume.as_str(),
                    AliasKind::TmuxPane.as_str(),
                    AliasKind::WatchPid.as_str(),
                    fresh_since,
                ],
                |r| {
                    Ok(LiveLocator {
                        session_id: SessionId::from(r.get::<_, String>(0)?),
                        harness_session_id: r.get::<_, Option<String>>(1)?,
                        resume_id: r.get::<_, Option<String>>(2)?,
                        tmux_pane: r.get::<_, Option<String>>(3)?,
                        watch_pid: r
                            .get::<_, Option<String>>(4)?
                            .and_then(|s| s.parse::<i32>().ok()),
                    })
                },
            )?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // ── canonical session aggregate: transitions ─────────────────────────────
    // Each transition is ONE txn that bumps state_version + updated_at and (when
    // public status changed) enqueues a status_outbox row. None of them bump the
    // version on a bare liveness refresh — that is `heartbeat_session`.

    /// Open a new turn: busy, fresh turn_id, cleared live activity. Also resets
    /// the PostToolUse debounce cursor (`turn_state.last_check_at`) so mid-turn
    /// deltas keep working. Returns the new snapshot (carries the turn_id the
    /// runtime must echo back to `apply_distill_result`). `None` if unknown.
    pub fn start_turn(&self, session_id: &str, ts: u64) -> Result<Option<SessionSnapshot>> {
        let n = self.conn.execute(
            "UPDATE session_state SET
               busy=1, phase='working', activity='',
               turn_id=turn_id+1, turn_started_at=?2,
               state_version=state_version+1, updated_at=?2, last_seen=?2
             WHERE session_id=?1",
            params![session_id, ts],
        )?;
        if n == 0 {
            return Ok(None);
        }
        // Keep the legacy turn_state cursor coherent for turn_check_due().
        self.conn.execute(
            "INSERT INTO turn_state (session_id, working, turn_started_at, last_check_at)
             VALUES (?1, 1, ?2, 0)
             ON CONFLICT(session_id) DO UPDATE SET working=1, turn_started_at=?2, last_check_at=0",
            params![session_id, ts],
        )?;
        self.enqueue_status_outbox_current(session_id, ts)?;
        self.local_session_snapshot(session_id)
    }

    /// Place a provisional title IFF none is set yet (title_source='none') AND
    /// `turn_id` still matches the current turn (so a stale seed can't apply).
    /// Returns the updated snapshot when it seeded, else `None`.
    pub fn seed_title_if_empty(
        &self,
        session_id: &str,
        turn_id: i64,
        title: &str,
        ts: u64,
    ) -> Result<Option<SessionSnapshot>> {
        let n = self.conn.execute(
            "UPDATE session_state SET
               title=?3, title_source='seed',
               state_version=state_version+1, updated_at=?4, last_seen=?4
             WHERE session_id=?1 AND turn_id=?2 AND title_source='none'",
            params![session_id, turn_id, title, ts],
        )?;
        if n == 0 {
            return Ok(None);
        }
        self.enqueue_status_outbox_current(session_id, ts)?;
        self.local_session_snapshot(session_id)
    }

    /// Apply a distilled (title, activity). Returns `None` (rejected) unless the
    /// session's CURRENT `(turn_id, state_version)` still equals
    /// `(base_turn_id, base_version)` — so a stale distill or a duplicate runtime
    /// structurally cannot flip the title.
    pub fn apply_distill_result(
        &self,
        session_id: &str,
        base_turn_id: i64,
        base_version: i64,
        title: &str,
        activity: &str,
        ts: u64,
    ) -> Result<Option<SessionSnapshot>> {
        let n = self.conn.execute(
            "UPDATE session_state SET
               title=?4, title_source='distill', activity=?5, last_distill_at=?6,
               state_version=state_version+1, updated_at=?6, last_seen=?6
             WHERE session_id=?1 AND turn_id=?2 AND state_version=?3",
            params![session_id, base_turn_id, base_version, title, activity, ts],
        )?;
        if n == 0 {
            return Ok(None);
        }
        self.enqueue_status_outbox_current(session_id, ts)?;
        self.local_session_snapshot(session_id)
    }

    /// Liveness refresh ONLY: bumps `last_seen`, never `state_version`/`updated_at`,
    /// never enqueues. The daemon re-arms the relay expiration by republishing the
    /// returned snapshot. Returns `None` if the session is unknown.
    pub fn heartbeat_session(&self, session_id: &str, ts: u64) -> Result<Option<SessionSnapshot>> {
        let n = self.conn.execute(
            "UPDATE session_state SET last_seen=?2 WHERE session_id=?1",
            params![session_id, ts],
        )?;
        if n == 0 {
            return Ok(None);
        }
        self.local_session_snapshot(session_id)
    }

    /// Close the turn: idle, live activity cleared, TITLE retained. Resets the
    /// debounce cursor. Bumps version + enqueues (busy changed).
    pub fn end_turn(&self, session_id: &str, ts: u64) -> Result<Option<SessionSnapshot>> {
        let Some(before) = self.local_session_snapshot(session_id)? else {
            return Ok(None);
        };

        if before.busy || !before.activity.is_empty() || before.phase != "idle" {
            self.conn.execute(
                "UPDATE session_state SET
                   busy=0, phase='idle', activity='',
                   state_version=state_version+1, updated_at=?2, last_seen=?2
                 WHERE session_id=?1",
                params![session_id, ts],
            )?;
            self.enqueue_status_outbox_current(session_id, ts)?;
        } else {
            self.conn.execute(
                "UPDATE session_state SET last_seen=?2 WHERE session_id=?1",
                params![session_id, ts],
            )?;
        }
        self.conn.execute(
            "INSERT INTO turn_state (session_id, working, turn_started_at, last_check_at)
             VALUES (?1, 0, 0, 0)
             ON CONFLICT(session_id) DO UPDATE SET working=0, last_check_at=0",
            params![session_id],
        )?;
        self.local_session_snapshot(session_id)
    }

    /// Finish the session: lifecycle='ended', idle, TITLE retained. The final
    /// publish still carries a fresh expiration; beats stop, so it ages off the
    /// relay after STATUS_TTL_SECS (no tombstone). Bumps version + enqueues.
    pub fn end_session(&self, session_id: &str, ts: u64) -> Result<Option<SessionSnapshot>> {
        let n = self.conn.execute(
            "UPDATE session_state SET
               busy=0, activity='', phase='idle', lifecycle='ended',
               state_version=state_version+1, updated_at=?2
             WHERE session_id=?1",
            params![session_id, ts],
        )?;
        if n == 0 {
            return Ok(None);
        }
        self.enqueue_status_outbox_current(session_id, ts)?;
        self.local_session_snapshot(session_id)
    }

    /// Retire a session a newer one replaced (lifecycle='superseded', idle).
    /// Bumps version + enqueues. Called internally by the registry's Supersede
    /// branch and exposed for the daemon's stale-sibling sweep.
    pub fn supersede_session(&self, session_id: &str, ts: u64) -> Result<Option<SessionSnapshot>> {
        let n = self.conn.execute(
            "UPDATE session_state SET
               busy=0, activity='', phase='idle', lifecycle='superseded',
               state_version=state_version+1, updated_at=?2
             WHERE session_id=?1",
            params![session_id, ts],
        )?;
        if n == 0 {
            return Ok(None);
        }
        self.enqueue_status_outbox_current(session_id, ts)?;
        self.local_session_snapshot(session_id)
    }

    // ── canonical session aggregate: read facade ──────────────────────────────

    /// The full snapshot of one local canonical session (any lifecycle).
    pub fn local_session_snapshot(&self, session_id: &str) -> Result<Option<SessionSnapshot>> {
        let sql = format!("SELECT {SESSION_STATE_COLS} FROM session_state WHERE session_id=?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![session_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_session_state(row)?)),
            None => Ok(None),
        }
    }

    /// Local sessions whose heartbeat is fresh (`last_seen>=since`) and lifecycle
    /// is active. `project=None` = all projects. `since=0` = include all.
    pub fn live_session_snapshots(
        &self,
        project: Option<&str>,
        since: u64,
    ) -> Result<Vec<SessionSnapshot>> {
        let sql = format!(
            "SELECT {SESSION_STATE_COLS} FROM session_state
             WHERE lifecycle='active' AND last_seen>=?1 AND (?2 IS NULL OR project=?2)
             ORDER BY last_seen DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, project], row_to_session_state)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Peer-mirror sessions seen at or after `since`. `project=None` = all.
    pub fn peer_session_snapshots(
        &self,
        project: Option<&str>,
        since: u64,
    ) -> Result<Vec<SessionSnapshot>> {
        let sql = format!(
            "SELECT {PEER_STATE_COLS} FROM peer_session_state
             WHERE last_seen>=?1 AND (?2 IS NULL OR project=?2)
             ORDER BY last_seen DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, project], row_to_peer_session_state)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// The shared delta query backing turn-start (subsequent turns) and the
    /// PostToolUse turn_check. Returns appeared / changed / finished-or-left
    /// transitions across BOTH `session_state` and `peer_session_state`, scoped
    /// to `project`, since cursor `since`, self-excluded by `exclude`. `now`
    /// drives liveness/expiry classification.
    ///
    /// A row surfaces when it appeared (`first_seen>=since`), changed
    /// (`updated_at>=since`, i.e. a versioned content change — agent went idle, a
    /// new title), or went gone (lifecycle ended/superseded since `since`, OR its
    /// liveness expired within the window). Pure-read: writes nothing.
    pub fn status_delta_since(
        &self,
        project: &str,
        since: u64,
        now: u64,
        exclude: Option<&str>,
    ) -> Result<Vec<StatusDeltaItem>> {
        let ttl = crate::domain::STATUS_TTL_SECS;
        // Window predicate (same on both tables): appeared OR changed OR
        // expired-within-window.
        let mut out: Vec<StatusDeltaItem> = Vec::new();

        let local_sql = format!(
            "SELECT {SESSION_STATE_COLS} FROM session_state
             WHERE project=?1
               AND (first_seen>=?2 OR updated_at>=?2 OR (last_seen < ?3 AND last_seen+?4 >= ?2))"
        );
        let now_minus_ttl = now.saturating_sub(ttl);
        // Self-echo dedup must cover EVERY local session's pubkey in this
        // project, not only those inside the delta window: a local session that
        // hasn't changed since the cursor still round-trips a fresh kind:30315
        // into peer_session_state, and that echo must never surface to ourselves.
        // Keyed on agent_pubkey since peer rows no longer carry a session_id (#5 §4).
        let mut local_pubkeys: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT agent_pubkey FROM session_state WHERE project=?1")?;
            let rows = stmt.query_map(params![project], |r| r.get::<_, String>(0))?;
            for pk in rows.filter_map(|r| r.ok()) {
                local_pubkeys.insert(pk);
            }
        }
        {
            let mut stmt = self.conn.prepare(&local_sql)?;
            let rows = stmt.query_map(
                params![project, since, now_minus_ttl, ttl],
                row_to_session_state,
            )?;
            for snap in rows.filter_map(|r| r.ok()) {
                if exclude == Some(snap.session_id.as_str()) {
                    continue;
                }
                if let Some(item) = classify_delta(snap, since, now) {
                    out.push(item);
                }
            }
        }

        let peer_sql = format!(
            "SELECT {PEER_STATE_COLS} FROM peer_session_state
             WHERE project=?1
               AND (first_seen>=?2 OR updated_at>=?2 OR (last_seen < ?3 AND last_seen+?4 >= ?2))"
        );
        {
            let mut stmt = self.conn.prepare(&peer_sql)?;
            let rows = stmt.query_map(
                params![project, since, now_minus_ttl, ttl],
                row_to_peer_session_state,
            )?;
            for snap in rows.filter_map(|r| r.ok()) {
                // exclude is a local session_id (te-*); peer snapshot uses pubkey
                // as session_id, so this guard only fires for the exact exclude match.
                if exclude == Some(snap.session_id.as_str()) {
                    continue;
                }
                // Dedup: skip peer echoes of our own sessions (keyed by pubkey).
                if local_pubkeys.contains(&snap.agent_pubkey) {
                    continue;
                }
                if let Some(item) = classify_delta(snap, since, now) {
                    out.push(item);
                }
            }
        }
        Ok(out)
    }

    // ── peer mirror write (kind:30315 materializer surface) ───────────────────

    /// Mirror an inbound kind:30315 into `peer_session_state`. Idempotent upsert
    /// keyed by `(pubkey, project)` — one row per agent per group; a newer
    /// heartbeat from the same agent replaces the older one. Bumps `state_version`
    /// + `updated_at` only when public content changed (title/activity/busy/host/
    /// rel_cwd/slug); advances `last_seen` only on a newer `emitted_at` so
    /// out-of-order refetches never resurrect a finished peer. `first_seen` is set
    /// once on insert. No native_session_id (issue #5 §4).
    pub fn record_peer_status(&self, obs: &PeerStatusObservation) -> Result<()> {
        let existing: Option<(String, String, i64, String, String, String, u64, i64)> = self
            .conn
            .query_row(
                "SELECT title, activity, busy, host, rel_cwd, agent_slug, last_seen, state_version
                 FROM peer_session_state
                 WHERE pubkey=?1 AND project=?2",
                params![obs.agent_pubkey, obs.project],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, u64>(6)?,
                        r.get::<_, i64>(7)?,
                    ))
                },
            )
            .ok();
        let busy_i = obs.busy as i64;
        match existing {
            None => {
                self.conn.execute(
                    "INSERT INTO peer_session_state
                       (pubkey, project, agent_slug, host, rel_cwd,
                        title, activity, busy, last_seen, state_version, lifecycle,
                        first_seen, updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,1,'active',?9,?10)",
                    params![
                        obs.agent_pubkey,
                        obs.project,
                        obs.agent_slug,
                        obs.host,
                        obs.rel_cwd,
                        obs.title,
                        obs.activity,
                        busy_i,
                        obs.emitted_at,
                        obs.observed_at,
                    ],
                )?;
            }
            Some((title, activity, busy, host, rel_cwd, slug, last_seen, version)) => {
                let content_changed = title != obs.title
                    || activity != obs.activity
                    || busy != busy_i
                    || host != obs.host
                    || rel_cwd != obs.rel_cwd
                    || (!obs.agent_slug.is_empty() && slug != obs.agent_slug);
                let new_seen = last_seen.max(obs.emitted_at);
                let new_version = if content_changed {
                    version + 1
                } else {
                    version
                };
                let new_updated = if content_changed {
                    obs.observed_at
                } else {
                    last_seen
                };
                self.conn.execute(
                    "UPDATE peer_session_state SET
                       agent_slug=CASE WHEN ?3<>'' THEN ?3 ELSE agent_slug END,
                       host=?4, rel_cwd=?5, title=?6, activity=?7, busy=?8,
                       last_seen=?9, state_version=?10, updated_at=?11
                     WHERE pubkey=?1 AND project=?2",
                    params![
                        obs.agent_pubkey,
                        obs.project,
                        obs.agent_slug,
                        obs.host,
                        obs.rel_cwd,
                        obs.title,
                        obs.activity,
                        busy_i,
                        new_seen,
                        new_version,
                        new_updated,
                    ],
                )?;
            }
        }
        Ok(())
    }

    // ── status outbox (publish queue) ─────────────────────────────────────────

    /// Enqueue an outbox row for the session's CURRENT `state_version`.
    fn enqueue_status_outbox_current(&self, session_id: &str, ts: u64) -> Result<()> {
        let version: Option<i64> = self
            .conn
            .query_row(
                "SELECT state_version FROM session_state WHERE session_id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(v) = version {
            self.enqueue_status_outbox(session_id, v, ts)?;
        }
        Ok(())
    }

    fn enqueue_status_outbox(&self, session_id: &str, state_version: i64, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO status_outbox
               (session_id, state_version, publish_state, retries, enqueued_at)
             VALUES (?1, ?2, 'pending', 0, ?3)",
            params![session_id, state_version, ts],
        )?;
        Ok(())
    }

    /// Pending publications joined to the CURRENT session snapshot, oldest first.
    /// The drainer publishes each via `Kind1Nip29Provider::set_status` and then
    /// calls `mark_status_published` / `mark_status_failed`.
    pub fn pending_status_outbox(&self, limit: u64) -> Result<Vec<StatusOutboxItem>> {
        let sql = format!(
            "SELECT o.session_id, o.state_version, o.retries, {cols}
             FROM status_outbox o
             JOIN session_state s ON s.session_id=o.session_id
             WHERE o.publish_state='pending'
             ORDER BY o.enqueued_at ASC, o.state_version ASC
             LIMIT ?1",
            cols = SESSION_STATE_COLS_PREFIXED
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let session_id: String = row.get(0)?;
            let state_version: i64 = row.get(1)?;
            let retries: i64 = row.get(2)?;
            // Snapshot columns start at index 3.
            let snapshot = row_to_session_state_offset(row, 3)?;
            Ok(StatusOutboxItem {
                session_id,
                state_version,
                retries,
                snapshot,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Mark a publication delivered, recording the native event id.
    pub fn mark_status_published(
        &self,
        session_id: &str,
        state_version: i64,
        native_event_id: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE status_outbox SET publish_state='published', native_event_id=?3, last_error=NULL
             WHERE session_id=?1 AND state_version=?2",
            params![session_id, state_version, native_event_id],
        )?;
        Ok(())
    }

    /// Record a failed publish attempt (increments retries, keeps it pending).
    pub fn mark_status_failed(
        &self,
        session_id: &str,
        state_version: i64,
        error: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE status_outbox SET retries=retries+1, last_error=?3
             WHERE session_id=?1 AND state_version=?2",
            params![session_id, state_version, error],
        )?;
        Ok(())
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
             ON CONFLICT(project) DO UPDATE SET name=?2, parent=?3, updated_at=?4",
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

    /// Returns `true` if `project` is known locally — either it has a
    /// `project_meta` row (seen on relay) or an `owned_groups` row (we created it).
    pub fn channel_exists(&self, project: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM project_meta WHERE project = ?1 \
                 UNION SELECT 1 FROM owned_groups WHERE project = ?1",
                params![project],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Mark an owned group as a per-session room (issue #6). Idempotent; assumes
    /// the group is already (or concurrently) recorded in `owned_groups`.
    pub fn mark_session_room(&self, project: &str, parent: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO owned_groups (project, created_at, is_session_room, room_parent)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(project) DO UPDATE SET is_session_room=1, room_parent=?3",
            params![project, ts, parent],
        )?;
        Ok(())
    }

    /// The work-root project a per-session room is nested under (set at mint).
    /// `None` if `project` is not a known per-session room. Materializer-safe.
    pub fn session_room_parent(&self, project: &str) -> Result<Option<String>> {
        let parent: rusqlite::Result<String> = self.conn.query_row(
            "SELECT room_parent FROM owned_groups WHERE project=?1 AND is_session_room=1",
            params![project],
            |r| r.get::<_, String>(0),
        );
        match parent {
            Ok(p) if !p.is_empty() => Ok(Some(p)),
            Ok(_) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// True if `project` is a per-session room (minted at session birth), as
    /// opposed to a project group or a task room.
    pub fn is_session_room(&self, project: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM owned_groups WHERE project=?1 AND is_session_room=1",
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

    /// Idempotently enqueue a live project chat row for one target session.
    /// Chat is intentionally per-session/live-only: callers decide the target
    /// sessions at materialization time and never catch up old chat on startup.
    pub fn enqueue_chat(&self, row: &ChatInboxRow) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO chat_inbox
               (chat_event_id, target_session, from_pubkey, from_slug, project, body, created_at, delivered, from_session, mentioned_session)
             VALUES (?1,?2,?3,?4,?5,?6,?7,0,?8,?9)",
            params![
                row.chat_event_id,
                row.target_session,
                row.from_pubkey,
                row.from_slug,
                row.project,
                row.body,
                row.created_at,
                row.from_session,
                row.mentioned_session,
            ],
        )?;
        Ok(changed > 0)
    }

    /// Idempotently record a local chat history row. This is separate from
    /// `chat_inbox`: the log powers explicit reads, while the inbox remains the
    /// live-only hook injection queue.
    pub fn record_chat(&self, row: &ChatLogRow) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO chat_messages
               (chat_event_id, from_pubkey, from_slug, host, project, body, created_at, from_session, mentioned_session)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                row.chat_event_id,
                row.from_pubkey,
                row.from_slug,
                row.host,
                row.project,
                row.body,
                row.created_at,
                row.from_session,
                row.mentioned_session,
            ],
        )?;
        Ok(changed > 0)
    }

    pub fn list_chat_messages(
        &self,
        project: &str,
        since: u64,
        limit: Option<u64>,
        offset: u64,
        tail: bool,
    ) -> Result<Vec<ChatLogRow>> {
        let limit = limit.unwrap_or(i64::MAX as u64).min(i64::MAX as u64) as i64;
        let offset = offset.min(i64::MAX as u64) as i64;
        let order = if tail {
            "created_at DESC, chat_event_id DESC"
        } else {
            "created_at ASC, chat_event_id ASC"
        };
        let sql = format!(
            "SELECT chat_event_id, from_pubkey, from_slug, host, project, body, created_at, from_session, mentioned_session
             FROM chat_messages
             WHERE project=?1 AND created_at>=?2
             ORDER BY {order}
             LIMIT ?3 OFFSET ?4"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows: Vec<ChatLogRow> = stmt
            .query_map(params![project, since, limit, offset], row_to_chat_log)?
            .filter_map(|r| r.ok())
            .collect();
        if tail {
            rows.reverse();
        }
        Ok(rows)
    }

    /// Recent project chat lines for tail backfill, newest first.
    /// `project = None` spans all projects. Each row is `(created_at, body,
    /// from_pubkey, project, from_session)` — enough to render a `Msg` event
    /// without a relay round-trip.
    pub fn recent_chat_for_backfill(
        &self,
        project: Option<&str>,
        since: u64,
        limit: u64,
    ) -> Result<Vec<(u64, String, String, String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT created_at, body, from_pubkey, project, from_session
             FROM chat_messages
             WHERE (?1 IS NULL OR project=?1) AND created_at >= ?2
             ORDER BY created_at DESC LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![project, since, limit as i64], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Read undelivered chat rows without marking them delivered. Used by
    /// mid-turn hook injection so the next turn-start remains authoritative.
    pub fn peek_chat(&self, session_id: &str) -> Result<Vec<ChatInboxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session, mentioned_session
             FROM chat_inbox WHERE target_session=?1 AND delivered=0 ORDER BY created_at",
        )?;
        let rows: Vec<ChatInboxRow> = stmt
            .query_map(params![session_id], row_to_chat)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Read undelivered chat rows that explicitly mention this session.
    pub fn peek_chat_mentions(&self, session_id: &str) -> Result<Vec<ChatInboxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session, mentioned_session
             FROM chat_inbox
             WHERE target_session=?1 AND mentioned_session=?1 AND delivered=0
             ORDER BY created_at",
        )?;
        let rows: Vec<ChatInboxRow> = stmt
            .query_map(params![session_id], row_to_chat)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return undelivered chat rows for a session and mark them delivered.
    pub fn drain_chat(&self, session_id: &str) -> Result<Vec<ChatInboxRow>> {
        let rows = self.peek_chat(session_id)?;
        self.conn.execute(
            "UPDATE chat_inbox SET delivered=1, delivered_at=?2 WHERE target_session=?1 AND delivered=0",
            params![session_id, crate::util::now_secs()],
        )?;
        Ok(rows)
    }

    /// Mark exactly these chat rows delivered for `session_id`.
    pub fn mark_chat_rows_delivered(
        &self,
        session_id: &str,
        event_ids: &[String],
        delivered_at: u64,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "UPDATE chat_inbox SET delivered=1, delivered_at=?3
             WHERE target_session=?1 AND chat_event_id=?2 AND delivered=0",
        )?;
        for event_id in event_ids {
            stmt.execute(params![session_id, event_id, delivered_at])?;
        }
        Ok(())
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
            "SELECT session_id, agent_slug, agent_pubkey, project, host, child_pid, watch_pid, created_at, alive, rel_cwd, channel
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

    /// Explicit chat mentions already drained to `session_id` at or after `since`.
    pub fn list_recently_delivered_chat_mentions(
        &self,
        session_id: &str,
        since: u64,
    ) -> Result<Vec<ChatInboxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_event_id, target_session, from_pubkey, from_slug, project, body, created_at, from_session, mentioned_session
             FROM chat_inbox
             WHERE target_session=?1 AND mentioned_session=?1 AND delivered=1 AND delivered_at>=?2
             ORDER BY created_at",
        )?;
        let rows: Vec<ChatInboxRow> = stmt
            .query_map(params![session_id, since], row_to_chat)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
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

    /// Record a distillation failure for this session (upserts — only the last
    /// error is kept in the DB; the log file retains full history).
    pub fn record_session_error(&self, session_id: &str, message: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO session_errors (session_id, message, ts) VALUES (?1, ?2, ?3)",
            rusqlite::params![session_id, message, ts],
        )?;
        Ok(())
    }

    /// Return the last distillation error for `session_id` if it occurred at or
    /// after `since` (unix seconds). Returns `None` when no recent error exists.
    pub fn get_recent_session_error(&self, session_id: &str, since: u64) -> Result<Option<String>> {
        let result: rusqlite::Result<String> = self.conn.query_row(
            "SELECT message FROM session_errors WHERE session_id = ?1 AND ts >= ?2",
            rusqlite::params![session_id, since],
            |row| row.get(0),
        );
        match result {
            Ok(msg) => Ok(Some(msg)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

mod endpoints;
pub use endpoints::SessionEndpoint;

// ── canonical session_state / peer_session_state helpers ─────────────────────

/// Canonical column order for `session_state` reads. Keep in lockstep with
/// `row_to_session_state`.
const SESSION_STATE_COLS: &str = "session_id, agent_slug, agent_pubkey, project, host, rel_cwd, \
     title, title_source, activity, busy, phase, turn_id, turn_started_at, last_distill_at, \
     last_seen, resume_id, state_version, lifecycle, first_seen, updated_at";

/// Same columns, table-qualified for the `status_outbox` join.
const SESSION_STATE_COLS_PREFIXED: &str = "s.session_id, s.agent_slug, s.agent_pubkey, s.project, \
     s.host, s.rel_cwd, s.title, s.title_source, s.activity, s.busy, s.phase, s.turn_id, \
     s.turn_started_at, s.last_distill_at, s.last_seen, s.resume_id, s.state_version, s.lifecycle, \
     s.first_seen, s.updated_at";

/// Canonical column order for `peer_session_state` reads. Keep in lockstep with
/// `row_to_peer_session_state`. Column indices (0-based):
///   0=pubkey, 1=project, 2=agent_slug, 3=host, 4=rel_cwd, 5=title,
///   6=activity, 7=busy, 8=last_seen, 9=state_version, 10=lifecycle,
///   11=first_seen, 12=updated_at
const PEER_STATE_COLS: &str = "pubkey, project, agent_slug, host, rel_cwd, \
     title, activity, busy, last_seen, state_version, lifecycle, first_seen, updated_at";

/// Mint a fresh canonical session id (daemon-owned, opaque).
fn mint_session_id() -> String {
    gen_id("te")
}

/// Build a `SessionSnapshot` from a `session_state` row whose columns start at 0.
fn row_to_session_state(row: &rusqlite::Row) -> rusqlite::Result<SessionSnapshot> {
    row_to_session_state_offset(row, 0)
}

/// Build a `SessionSnapshot` from a `session_state` row whose first column is at
/// `base` (used by the outbox join, where leading columns precede the snapshot).
fn row_to_session_state_offset(
    row: &rusqlite::Row,
    base: usize,
) -> rusqlite::Result<SessionSnapshot> {
    Ok(SessionSnapshot {
        source: SnapshotSource::Local,
        session_id: SessionId::from(row.get::<_, String>(base)?),
        agent_slug: row.get(base + 1)?,
        agent_pubkey: row.get(base + 2)?,
        project: row.get(base + 3)?,
        host: row.get(base + 4)?,
        rel_cwd: row.get(base + 5)?,
        title: row.get(base + 6)?,
        title_source: TitleSource::from_str(&row.get::<_, String>(base + 7)?),
        activity: row.get(base + 8)?,
        busy: row.get::<_, i64>(base + 9)? != 0,
        phase: row.get(base + 10)?,
        turn_id: row.get(base + 11)?,
        turn_started_at: row.get(base + 12)?,
        last_distill_at: row.get(base + 13)?,
        last_seen: row.get(base + 14)?,
        resume_id: row.get(base + 15)?,
        state_version: row.get(base + 16)?,
        lifecycle: Lifecycle::from_str(&row.get::<_, String>(base + 17)?),
        first_seen: row.get(base + 18)?,
        updated_at: row.get(base + 19)?,
    })
}

/// Build a `SessionSnapshot` (Peer source) from a `peer_session_state` row. Peer
/// rows carry no turn/distill/resume data, so those fields project as defaults.
/// The PK is (pubkey, project) — `session_id` is synthesized as `pubkey` since
/// there is no native_session_id (issue #5 §4).
fn row_to_peer_session_state(row: &rusqlite::Row) -> rusqlite::Result<SessionSnapshot> {
    // 0=pubkey, 1=project, 2=agent_slug, 3=host, 4=rel_cwd, 5=title,
    // 6=activity, 7=busy, 8=last_seen, 9=state_version, 10=lifecycle,
    // 11=first_seen, 12=updated_at
    let busy = row.get::<_, i64>(7)? != 0;
    Ok(SessionSnapshot {
        source: SnapshotSource::Peer,
        agent_pubkey: row.get(0)?,
        project: row.get(1)?,
        // No native session id: use pubkey as the stable per-agent identity.
        session_id: SessionId::from(row.get::<_, String>(0)?),
        agent_slug: row.get(2)?,
        host: row.get(3)?,
        rel_cwd: row.get(4)?,
        title: row.get(5)?,
        title_source: if row.get::<_, String>(5)?.is_empty() {
            TitleSource::None
        } else {
            TitleSource::Peer
        },
        activity: row.get(6)?,
        busy,
        phase: if busy {
            "working".into()
        } else {
            "idle".into()
        },
        turn_id: 0,
        turn_started_at: 0,
        last_distill_at: 0,
        last_seen: row.get(8)?,
        resume_id: String::new(),
        state_version: row.get(9)?,
        lifecycle: Lifecycle::from_str(&row.get::<_, String>(10)?),
        first_seen: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

/// Classify one in-window snapshot into an appeared/changed/gone delta, or
/// `None` when it doesn't qualify. Gone takes precedence (ended/superseded since
/// the cursor, or liveness expired within the window); then appeared
/// (first_seen>=since and still live); then changed (updated_at>=since and live).
fn classify_delta(snap: SessionSnapshot, since: u64, now: u64) -> Option<StatusDeltaItem> {
    let ttl = crate::domain::STATUS_TTL_SECS;
    let derived = derive_status(&snap, now);
    let live = derived.liveness.is_live();
    let was_live_at_since = snap.last_seen.saturating_add(ttl) >= since;
    let expired_in_window = !live && was_live_at_since && now.saturating_sub(snap.last_seen) > ttl;

    let kind = if (!snap.lifecycle.is_active() && snap.updated_at >= since) || expired_in_window {
        DeltaKind::Gone
    } else if snap.first_seen >= since && live {
        DeltaKind::Appeared
    } else if snap.updated_at >= since && live {
        DeltaKind::Changed
    } else {
        return None;
    };
    Some(StatusDeltaItem {
        kind,
        snapshot: snap,
        derived,
    })
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
        channel: row.get(10)?,
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

fn row_to_chat(row: &rusqlite::Row) -> rusqlite::Result<ChatInboxRow> {
    Ok(ChatInboxRow {
        chat_event_id: row.get(0)?,
        target_session: row.get(1)?,
        from_pubkey: row.get(2)?,
        from_slug: row.get(3)?,
        project: row.get(4)?,
        body: row.get(5)?,
        created_at: row.get(6)?,
        from_session: row.get(7)?,
        mentioned_session: row.get(8)?,
    })
}

fn row_to_chat_log(row: &rusqlite::Row) -> rusqlite::Result<ChatLogRow> {
    Ok(ChatLogRow {
        chat_event_id: row.get(0)?,
        from_pubkey: row.get(1)?,
        from_slug: row.get(2)?,
        host: row.get(3)?,
        project: row.get(4)?,
        body: row.get(5)?,
        created_at: row.get(6)?,
        from_session: row.get(7)?,
        mentioned_session: row.get(8)?,
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
            channel: String::new(),
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

    /// A session that registers, starts a turn, then ends a turn surfaces in
    /// `status_delta_since` as Changed; a freshly registered one as Appeared; an
    /// ended one as Gone. Project-scoped + self-excluded.
    #[test]
    fn status_delta_since_classifies_appeared_changed_gone() {
        use crate::session::{DeltaKind, Harness, SessionObservation};
        let s = Store::open_memory().unwrap();
        let mk = |slug: &str, pk: &str, proj: &str, ts: u64| SessionObservation {
            agent_slug: slug.into(),
            agent_pubkey: pk.into(),
            project: proj.into(),
            host: "host".into(),
            rel_cwd: String::new(),
            harness: Harness::ClaudeCode,
            harness_session_id: Some(format!("h-{slug}")),
            resume_id: None,
            tmux_pane: None,
            watch_pid: None,
            observed_at: ts,
        };
        // Registered before the cursor → not "appeared", but a turn change after.
        let a = s
            .register_or_reassert_session(&mk("alpha", "pk-a", "proj", 100))
            .unwrap();
        // Registered AFTER the cursor → appeared.
        let now = 200u64;
        let since = 150u64;
        let b = s
            .register_or_reassert_session(&mk("bravo", "pk-b", "proj", 160))
            .unwrap();
        // Different project → excluded.
        let _ = s
            .register_or_reassert_session(&mk("gamma", "pk-c", "other", 160))
            .unwrap();
        // alpha changes after the cursor.
        s.start_turn(a.session_id.as_str(), 170).unwrap();

        let delta = s
            .status_delta_since("proj", since, now, Some(b.session_id.as_str()))
            .unwrap();
        // bravo is excluded; alpha must be present as Changed.
        assert!(delta
            .iter()
            .any(|d| d.snapshot.session_id == a.session_id && d.kind == DeltaKind::Changed));
        assert!(delta.iter().all(|d| d.snapshot.project == "proj"));

        // End alpha's session → it surfaces as Gone.
        s.end_session(a.session_id.as_str(), 180).unwrap();
        let delta2 = s.status_delta_since("proj", since, now, None).unwrap();
        assert!(delta2
            .iter()
            .any(|d| d.snapshot.session_id == a.session_id && d.kind == DeltaKind::Gone));
    }

    /// A local session whose own kind:30315 round-trips back from the relay into
    /// `peer_session_state` MUST surface in the delta exactly ONCE. Before the
    /// dedup, the local row and its peer echo were both emitted, producing the
    /// duplicated (mirrored) lines in the turn-start fabric block.
    #[test]
    fn status_delta_since_dedups_local_session_peer_echo() {
        use crate::session::{Harness, PeerStatusObservation, SessionObservation};
        let s = Store::open_memory().unwrap();
        let local = s
            .register_or_reassert_session(&SessionObservation {
                agent_slug: "alpha".into(),
                agent_pubkey: "pk-a".into(),
                project: "proj".into(),
                host: "host".into(),
                rel_cwd: String::new(),
                harness: Harness::ClaudeCode,
                harness_session_id: Some("h-alpha".into()),
                resume_id: None,
                tmux_pane: None,
                watch_pid: None,
                observed_at: 160,
            })
            .unwrap();
        // The same agent's status, observed back off the relay as a peer echo.
        // Keyed by (pubkey, project) — no session_id on the peer row.
        s.record_peer_status(&PeerStatusObservation {
            agent_pubkey: "pk-a".into(),
            agent_slug: "alpha".into(),
            project: "proj".into(),
            host: "host".into(),
            rel_cwd: String::new(),
            title: String::new(),
            activity: String::new(),
            busy: false,
            emitted_at: 165,
            observed_at: 165,
        })
        .unwrap();

        let delta = s.status_delta_since("proj", 150, 200, None).unwrap();
        // The local row surfaces; the peer echo (same pubkey) is deduped out.
        let hits = delta
            .iter()
            .filter(|d| d.snapshot.agent_pubkey == "pk-a")
            .count();
        assert_eq!(
            hits, 1,
            "local session + its own peer echo must dedup to one (keyed by pubkey)"
        );
    }

    /// A session is never told about its own status: even when its own kind:30315
    /// has round-tripped into `peer_session_state`, passing the session as
    /// `exclude` drops BOTH the local row and the peer echo.
    #[test]
    fn status_delta_since_excludes_self_even_with_peer_echo() {
        use crate::session::{Harness, PeerStatusObservation, SessionObservation};
        let s = Store::open_memory().unwrap();
        let me = s
            .register_or_reassert_session(&SessionObservation {
                agent_slug: "me".into(),
                agent_pubkey: "pk-me".into(),
                project: "proj".into(),
                host: "host".into(),
                rel_cwd: String::new(),
                harness: Harness::ClaudeCode,
                harness_session_id: Some("h-me".into()),
                resume_id: None,
                tmux_pane: None,
                watch_pid: None,
                observed_at: 160,
            })
            .unwrap();
        s.record_peer_status(&PeerStatusObservation {
            agent_pubkey: "pk-me".into(),
            agent_slug: "me".into(),
            project: "proj".into(),
            host: "host".into(),
            rel_cwd: String::new(),
            title: String::new(),
            activity: String::new(),
            busy: false,
            emitted_at: 165,
            observed_at: 165,
        })
        .unwrap();

        let delta = s
            .status_delta_since("proj", 150, 200, Some(me.session_id.as_str()))
            .unwrap();
        // The local row is excluded by session_id; the peer echo is deduped by
        // pubkey since "pk-me" appears in local_pubkeys.
        assert!(
            delta.iter().all(|d| d.snapshot.agent_pubkey != "pk-me"),
            "a session must never see its own status (local row or peer echo)"
        );
    }

    /// A still-`active` session whose heartbeats stopped (no event for > TTL)
    /// MUST surface as `Gone` (liveness expired within the window) — a session
    /// that drops off the relay stays reportable as gone, never silently lingers.
    #[test]
    fn status_delta_since_reports_expired_session_as_gone() {
        use crate::session::{DeltaKind, Harness, SessionObservation};
        let s = Store::open_memory().unwrap();
        let obs = SessionObservation {
            agent_slug: "ghost".into(),
            agent_pubkey: "pk-ghost".into(),
            project: "proj".into(),
            host: "host".into(),
            rel_cwd: String::new(),
            harness: Harness::ClaudeCode,
            harness_session_id: Some("h-ghost".into()),
            resume_id: None,
            tmux_pane: None,
            watch_pid: None,
            observed_at: 100,
        };
        // Registered + last seen at t=100, then never heard from again.
        let ghost = s.register_or_reassert_session(&obs).unwrap();
        // `now` is far past last_seen + STATUS_TTL_SECS; the cursor is between the
        // last sighting and now, so the expiry falls inside the window.
        let now = 100 + crate::domain::STATUS_TTL_SECS + 200;
        let since = 100 + crate::domain::STATUS_TTL_SECS / 2;
        let delta = s.status_delta_since("proj", since, now, None).unwrap();
        let item = delta
            .iter()
            .find(|d| d.snapshot.session_id == ghost.session_id)
            .expect("expired session must still surface in the delta");
        assert_eq!(item.kind, DeltaKind::Gone, "expired session must be Gone");
        assert!(
            !item.derived.liveness.is_live(),
            "an expired session is never live"
        );
    }

    #[test]
    fn opencode_reassert_with_echoed_canonical_id_reattaches_not_supersedes() {
        use crate::session::{Harness, SessionObservation};
        let s = Store::open_memory().unwrap();
        // session-start: opencode owns no native id (echo harness), so the daemon
        // mints the canonical id, anchored on pane + watched pid. No
        // harness_session_id / resume_id yet.
        let start = SessionObservation {
            agent_slug: "opencode".into(),
            agent_pubkey: "pk-oc".into(),
            project: "proj".into(),
            host: "host".into(),
            rel_cwd: String::new(),
            harness: Harness::Opencode,
            harness_session_id: None,
            resume_id: None,
            tmux_pane: Some("%0".into()),
            watch_pid: Some(70282),
            observed_at: 100,
        };
        let canonical = s
            .register_or_reassert_session(&start)
            .unwrap()
            .session_id
            .as_str()
            .to_string();

        // user-prompt-submit: the plugin echoes the canonical id back as the
        // harness session id, now knows opencode's resume token, and reports a
        // DIFFERENT pid (ancestor search). Pre-fix this missed the alias lookup and
        // fell through to the pane/pid supersede branch, minting a brand-new
        // session on every first turn.
        let reassert = SessionObservation {
            agent_slug: "opencode".into(),
            agent_pubkey: "pk-oc".into(),
            project: "proj".into(),
            host: "host".into(),
            rel_cwd: String::new(),
            harness: Harness::Opencode,
            harness_session_id: Some(canonical.clone()),
            resume_id: Some("ses_abc".into()),
            tmux_pane: Some("%0".into()),
            watch_pid: Some(99999),
            observed_at: 160,
        };
        let after = s
            .register_or_reassert_session(&reassert)
            .unwrap()
            .session_id
            .as_str()
            .to_string();
        assert_eq!(
            after, canonical,
            "reassert must reattach to the same canonical session, not mint a new one"
        );
        // No churn: exactly one session_state row exists (pre-fix the reassert
        // superseded into a second row, leaving one ended + one active).
        let rows: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM session_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            rows, 1,
            "exactly one session_state row (no churn), got {rows}"
        );
    }

    #[test]
    fn turn_check_due_gates_and_advances_cursor() {
        let s = Store::open_memory().unwrap();
        // Not in a turn → never due (avoids querying all history).
        assert_eq!(s.turn_check_due("sess", 1000, 60).unwrap(), None);

        // Turn starts at t=1000; first check at t=1000 is due, since=turn start.
        s.mark_turn_start("sess", 1000).unwrap();
        assert_eq!(s.turn_check_due("sess", 1000, 60).unwrap(), Some(1000));

        // Within the 60s floor of the last check → suppressed.
        assert_eq!(s.turn_check_due("sess", 1059, 60).unwrap(), None);

        // 60s elapsed → due again, since = the previous check time (1000).
        assert_eq!(s.turn_check_due("sess", 1060, 60).unwrap(), Some(1000));
        // Cursor advanced to 1060: the next window starts there.
        assert_eq!(s.turn_check_due("sess", 1130, 60).unwrap(), Some(1060));

        // A new turn resets the cursor → first check is due immediately again.
        s.mark_turn_start("sess", 2000).unwrap();
        assert_eq!(s.turn_check_due("sess", 2000, 60).unwrap(), Some(2000));

        // Turn end clears working/cursor → not in a turn → not due.
        s.mark_turn_end("sess").unwrap();
        assert_eq!(s.turn_check_due("sess", 3000, 60).unwrap(), None);
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
    fn session_room_flag_roundtrip() {
        let s = Store::open_memory().unwrap();
        // A plain owned group is not a session room.
        s.mark_group_owned("proj", 100).unwrap();
        assert!(!s.is_session_room("proj").unwrap());
        // Marking a room sets the flag + records its work-root parent; it stays owned.
        s.mark_session_room("proj-room", "proj", 100).unwrap();
        assert!(s.is_session_room("proj-room").unwrap());
        assert!(s.is_group_owned("proj-room").unwrap());
        assert_eq!(s.session_room_parent("proj-room").unwrap().as_deref(), Some("proj"));
        // Idempotent (parent preserved).
        s.mark_session_room("proj-room", "proj", 200).unwrap();
        assert!(s.is_session_room("proj-room").unwrap());
        // Unknown group is not a session room and has no parent.
        assert!(!s.is_session_room("nope").unwrap());
        assert_eq!(s.session_room_parent("nope").unwrap(), None);
    }

    #[test]
    fn group_parent_distinguishes_subgroup_from_project() {
        let s = Store::open_memory().unwrap();
        // Unknown group → no parent.
        assert_eq!(s.group_parent("unknown").unwrap(), None);
        // Top-level project (empty parent) → None.
        s.upsert_group_metadata("proj", "Proj", "", 100).unwrap();
        assert_eq!(s.group_parent("proj").unwrap(), None);
        // Subgroup with a parent → Some(parent).
        s.upsert_group_metadata("proj-room", "Room", "proj", 100)
            .unwrap();
        assert_eq!(s.group_parent("proj-room").unwrap(), Some("proj".into()));
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

    // ── Phase 1: canonical read-model schema ─────────────────────────────

    #[test]
    fn phase1_new_tables_exist_after_open() {
        let s = Store::open_memory().unwrap();
        let n: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                 ('projects','project_origins','inbound_quarantine','membership')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 4, "all 4 Phase 1 tables must be created");
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

    /// record_peer_status mirrors a kind:30315 into peer_session_state and bumps
    /// state_version only on content change.
    #[test]
    fn record_peer_status_upserts_and_versions() {
        use crate::session::PeerStatusObservation;
        let s = Store::open_memory().unwrap();
        let mut obs = PeerStatusObservation {
            agent_pubkey: "pk-peer".into(),
            agent_slug: "peer".into(),
            project: "proj".into(),
            host: "host2".into(),
            rel_cwd: String::new(),
            title: "fixing auth".into(),
            activity: "editing".into(),
            busy: true,
            emitted_at: 100,
            observed_at: 100,
        };
        s.record_peer_status(&obs).unwrap();
        let snaps = s.peer_session_snapshots(Some("proj"), 0).unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].title, "fixing auth");
        assert_eq!(snaps[0].state_version, 1);
        // Same content, newer emit → no version bump, fresher last_seen.
        obs.emitted_at = 130;
        obs.observed_at = 130;
        s.record_peer_status(&obs).unwrap();
        let snaps = s.peer_session_snapshots(Some("proj"), 0).unwrap();
        assert_eq!(snaps[0].state_version, 1);
        assert_eq!(snaps[0].last_seen, 130);
        // Content change → version bump.
        obs.busy = false;
        obs.activity = String::new();
        obs.emitted_at = 160;
        obs.observed_at = 160;
        s.record_peer_status(&obs).unwrap();
        let snaps = s.peer_session_snapshots(Some("proj"), 0).unwrap();
        assert_eq!(snaps[0].state_version, 2);
        assert!(!snaps[0].busy);
    }

    /// register_or_reassert_session: alias hit reasserts the same canonical id;
    /// a fresh harness id mints a new one.
    #[test]
    fn register_session_alias_hit_reasserts() {
        use crate::session::{Harness, SessionObservation};
        let s = Store::open_memory().unwrap();
        let obs = |sid: &str, ts: u64| SessionObservation {
            agent_slug: "claude".into(),
            agent_pubkey: "pk".into(),
            project: "proj".into(),
            host: "host".into(),
            rel_cwd: String::new(),
            harness: Harness::ClaudeCode,
            harness_session_id: Some(sid.into()),
            resume_id: None,
            tmux_pane: None,
            watch_pid: None,
            observed_at: ts,
        };
        let a = s.register_or_reassert_session(&obs("h1", 10)).unwrap();
        let a2 = s.register_or_reassert_session(&obs("h1", 20)).unwrap();
        assert_eq!(
            a.session_id, a2.session_id,
            "same harness id → same canonical id"
        );
        assert_eq!(
            a2.state_version, a.state_version,
            "identical reassert refreshes liveness without a public version bump"
        );
        assert_eq!(a2.last_seen, 20, "reassert refreshes liveness");
        let b = s.register_or_reassert_session(&obs("h2", 30)).unwrap();
        assert_ne!(
            a.session_id, b.session_id,
            "new harness id → new canonical id"
        );
    }

    /// all_live_local_snapshots feeds the heartbeat expiration re-arm: it must
    /// return live sessions whose last_seen is fresh, drop stale ones, and drop
    /// ended ones — otherwise live-but-idle sessions age off the relay.
    #[test]
    fn all_live_local_snapshots_filters_fresh_and_active() {
        use crate::session::{Harness, SessionObservation};
        let s = Store::open_memory().unwrap();
        let obs = SessionObservation {
            agent_slug: "claude".into(),
            agent_pubkey: "pk".into(),
            project: "proj".into(),
            host: "host".into(),
            rel_cwd: String::new(),
            harness: Harness::ClaudeCode,
            harness_session_id: Some("h1".into()),
            resume_id: None,
            tmux_pane: None,
            watch_pid: None,
            observed_at: 1000,
        };
        let snap = s.register_or_reassert_session(&obs).unwrap();
        s.heartbeat_session(snap.session_id.as_str(), 1000).ok();

        // Fresh window includes it; a window past its last_seen excludes it.
        assert_eq!(
            s.all_live_local_snapshots(910).unwrap().len(),
            1,
            "fresh → included"
        );
        assert!(
            s.all_live_local_snapshots(1001).unwrap().is_empty(),
            "stale → excluded"
        );

        // Ending the session drops it from the live set (lifecycle != active).
        s.end_session(snap.session_id.as_str(), 1000).ok();
        assert!(
            s.all_live_local_snapshots(910).unwrap().is_empty(),
            "ended → excluded even when last_seen is fresh"
        );
    }

    /// versioned distill guard: a stale base_version is rejected.
    #[test]
    fn apply_distill_result_rejects_stale_version() {
        use crate::session::{Harness, SessionObservation};
        let s = Store::open_memory().unwrap();
        let snap = s
            .register_or_reassert_session(&SessionObservation {
                agent_slug: "claude".into(),
                agent_pubkey: "pk".into(),
                project: "proj".into(),
                host: "host".into(),
                rel_cwd: String::new(),
                harness: Harness::ClaudeCode,
                harness_session_id: Some("h1".into()),
                resume_id: None,
                tmux_pane: None,
                watch_pid: None,
                observed_at: 10,
            })
            .unwrap();
        let turn = s.start_turn(snap.session_id.as_str(), 20).unwrap().unwrap();
        // Wrong base_version → rejected.
        assert!(s
            .apply_distill_result(
                turn.session_id.as_str(),
                turn.turn_id,
                turn.state_version + 99,
                "T",
                "A",
                30
            )
            .unwrap()
            .is_none());
        // Correct (turn_id, state_version) → applied.
        let applied = s
            .apply_distill_result(
                turn.session_id.as_str(),
                turn.turn_id,
                turn.state_version,
                "Distilled",
                "doing",
                30,
            )
            .unwrap();
        assert_eq!(applied.unwrap().title, "Distilled");
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

    // ── Phase 6 dual-write tests ──────────────────────────────────────────────

    // ── Phase 7 tests ─────────────────────────────────────────────────────────
}
