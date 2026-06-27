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
use rusqlite::{params, Connection, OptionalExtension};
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

impl SessionRecord {
    /// The NIP-29 group id this session currently routes under — its channel
    /// when set, else its per-session room (`project`). All fabric publishing
    /// (chat/mentions/proposals), local chat routing, `who`/statusline scoping,
    /// and turn-context deltas key on this so `channels switch` actually moves
    /// the session to a different room without restarting. `project` alone is
    /// stale the moment `channel` is set.
    pub fn route_scope(&self) -> &str {
        if self.channel.is_empty() {
            &self.project
        } else {
            &self.channel
        }
    }
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

/// A durable ordinal identity's binding to a room + (when live) a harness
/// session (issue #47). One row per `(pubkey, h)`; the authoritative resume key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRoute {
    pub pubkey: String,
    pub h: String,
    pub session_id: String,
    pub base_pubkey: String,
    pub agent_slug: String,
    pub ordinal: u32,
    pub label: String,
    /// Harness kind ("claude-code"/"codex"/"opencode") for the bound session;
    /// drives the resume command shape. Empty when never bound to a harness.
    pub harness_kind: String,
    /// Native harness session id used for `--resume`. Empty when unknown.
    pub native_id: String,
    pub alive: bool,
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

/// One pending kind:30315 publication, returned by `pending_status_outbox`.
/// `snapshot` is the CURRENT `session_state` row for `session_id` (the drainer
/// publishes the latest fact; older pending versions coalesce). The drainer
/// builds a `Status` from `snapshot`, sets `expires_at = now + STATUS_TTL_SECS`,
/// calls `Nip29Provider::set_status`, then `mark_status_published`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusOutboxItem {
    pub session_id: String,
    pub state_version: i64,
    pub retries: i64,
    pub snapshot: SessionSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusOutboxDebugRow {
    pub session_id: String,
    pub state_version: i64,
    pub publish_state: String,
    pub retries: i64,
    pub native_event_id: Option<String>,
    pub last_error: Option<String>,
    pub enqueued_at: u64,
    pub agent_slug: String,
    pub project: String,
    pub title: String,
    pub activity: String,
    pub busy: bool,
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

mod schema;
use schema::SCHEMA;

mod chat;
mod core;
mod groups;
mod local_sessions;
mod ordinals;
mod peers;
mod read_models;
mod session_registry;
mod session_transitions;
mod turn_state;

mod channels;
mod endpoints;
pub use endpoints::SessionEndpoint;

mod membership;

mod outbox;
mod presence;

mod quarantine;
pub use quarantine::QuarantinedEnvelope;

// ── canonical session_state helpers ──────────────────────────────────────────

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
#[path = "state/tests_sessions.rs"]
mod tests_sessions;

#[cfg(test)]
#[path = "state/tests_groups.rs"]
mod tests_groups;

#[cfg(test)]
#[path = "state/tests_read_models.rs"]
mod tests_read_models;

#[cfg(test)]
#[path = "state/tests_ordinals.rs"]
mod tests_ordinals;
