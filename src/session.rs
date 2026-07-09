//! Pure session-aggregate types + identity resolution + status derivation.
//!
//! This module is the *shape* of a session: the canonical id the daemon mints,
//! the identity inputs the hook observes, the complete public snapshot every
//! reader projects from, and the pure helpers that decide identity and derive
//! liveness. It names NO Nostr concepts (no kinds, tags, relays) and performs
//! NO I/O — `state.rs` owns the SQLite side, `domain.rs` owns the wire-facing
//! `Status`. Everything here is deterministic and table-testable.
//!
//! Local-only runtime truth lives in the `sessions` table (pid, harness ids,
//! transcript, current draft status to publish). Published actor status is
//! projected into `relay_status` from relay-confirmed kind:30315 events. A
//! `SessionSnapshot` is the shared in-memory projection of both, so one
//! `derive_status` serves local and relay-confirmed readers identically.

use crate::domain::Lifecycle;

// The canonical session id IS `util::SessionId`. The daemon mints it; harness
// ids / resume tokens / hosted endpoints become aliases that resolve to it. Re-exported
// so downstream code can say `session::SessionId` for the canonical id.
pub use crate::domain::STATUS_TTL_SECS;
pub use crate::util::SessionId;

// ── harness + alias taxonomy ─────────────────────────────────────────────────

/// Which agent harness produced an observation. The string form is what the
/// hook layer passes and what `session_aliases.harness` stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Harness {
    ClaudeCode,
    Codex,
    Opencode,
    Grok,
    Unknown,
}

impl Harness {
    pub fn as_str(&self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Codex => "codex",
            Harness::Opencode => "opencode",
            Harness::Grok => "grok",
            Harness::Unknown => "unknown",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "claude-code" | "claude" => Harness::ClaudeCode,
            "codex" => Harness::Codex,
            "opencode" => Harness::Opencode,
            "grok" => Harness::Grok,
            _ => Harness::Unknown,
        }
    }
}

/// The kind of external identifier carried by a `session_aliases` row. Together
/// with `harness` + the raw `external_id` it forms the alias PK, so the same
/// endpoint id under two harnesses (or a resume token vs a harness-native id)
/// never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AliasKind {
    /// The harness-native session id (claude/codex adopt it; opencode `ses_*`).
    HarnessSession,
    /// A `--resume` token distinct from the harness id (opencode).
    Resume,
    /// A hosted PTY session id.
    PtySession,
    /// The watched host PID, stringified.
    WatchPid,
    /// A daemon-generated `te-*` id (when the harness supplied none).
    Generated,
}

impl AliasKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AliasKind::HarnessSession => "harness",
            AliasKind::Resume => "resume",
            AliasKind::PtySession => "pty_session",
            AliasKind::WatchPid => "watch_pid",
            AliasKind::Generated => "generated",
        }
    }
}

/// Where a `title` came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSource {
    None,
    /// Produced by the LLM distiller.
    Distill,
    /// Mirrored from a peer's kind:30315 wire title.
    Peer,
}

impl TitleSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            TitleSource::None => "none",
            TitleSource::Distill => "distill",
            TitleSource::Peer => "peer",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "distill" => TitleSource::Distill,
            "peer" => TitleSource::Peer,
            _ => TitleSource::None,
        }
    }
}

// ── identity key + observations ──────────────────────────────────────────────

/// The full identity-input tuple for a session. The daemon never keys storage
/// on this directly (the canonical `SessionId` is the PK); it is the set of
/// signals `resolve_identity` consults to decide mint vs reattach vs supersede.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKey {
    pub host: String,
    pub channel: String,
    pub agent_pubkey: String,
    pub agent_slug: String,
    pub harness: Harness,
    /// Harness-native session id, when the harness supplies one.
    pub harness_session_id: Option<String>,
    /// Resume token (claude/codex == harness id; opencode `ses_*`).
    pub resume_id: Option<String>,
    /// Hosted PTY session id, when launched through tenex-edge.
    pub pty_session: Option<String>,
    /// Watched host PID.
    pub watch_pid: Option<i32>,
}

/// The normalized observation `cli/hooks.rs` -> `rpc_session_start` hands to
/// `Store::register_or_reassert_session`. It reports *facts the hook saw*, never
/// an identity decision — the registry owns identity policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObservation {
    pub agent_slug: String,
    pub agent_pubkey: String,
    pub channel: String,
    pub host: String,
    pub rel_cwd: String,
    pub harness: Harness,
    pub harness_session_id: Option<String>,
    pub resume_id: Option<String>,
    pub pty_session: Option<String>,
    pub watch_pid: Option<i32>,
    /// Wall-clock seconds the hook fired; becomes `last_seen`/`first_seen`.
    pub observed_at: u64,
}

impl SessionObservation {
    /// Channel the identity inputs out of the observation.
    pub fn key(&self) -> SessionKey {
        SessionKey {
            host: self.host.clone(),
            channel: self.channel.clone(),
            agent_pubkey: self.agent_pubkey.clone(),
            agent_slug: self.agent_slug.clone(),
            harness: self.harness,
            harness_session_id: self.harness_session_id.clone(),
            resume_id: self.resume_id.clone(),
            pty_session: self.pty_session.clone(),
            watch_pid: self.watch_pid,
        }
    }
}

/// What `Store::upsert_status` receives from the kind:30315 materializer.
/// Mirrors a relay-confirmed self-contained status signal into `relay_status`.
/// Keyed by `(agent_pubkey, channel)` — one row per actor per group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerStatusObservation {
    pub agent_pubkey: String,
    /// Resolved from the peer's kind:0 profile (NEVER self-asserted); may be "".
    pub agent_slug: String,
    /// Group id (== kind:30315 `d` tag == `h` tag == channel slug).
    pub channel: String,
    pub host: String,
    pub rel_cwd: String,
    pub title: String,
    pub activity: String,
    pub busy: bool,
    /// Event `created_at` — drives liveness (a finished peer stops emitting).
    pub emitted_at: u64,
    /// Local ingest time.
    pub observed_at: u64,
}

// ── snapshot (the complete public state) ─────────────────────────────────────

/// Whether a snapshot came from this machine's local runtime draft or the
/// relay-confirmed published presence projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotSource {
    Local,
    Peer,
}

/// The complete public state of ONE actor — the unified projection of both a
/// local `sessions` row and a relay-confirmed `relay_status` row. Every
/// reader (`who`, statusline, turn deltas, the outbox drainer) consumes this
/// exact shape and runs `derive_status` over it, so the busy/liveness fork is
/// structurally impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub source: SnapshotSource,
    /// Canonical id for Local; the peer's native id for Peer.
    pub session_id: SessionId,
    pub agent_pubkey: String,
    pub agent_slug: String,
    pub channel: String,
    pub host: String,
    pub rel_cwd: String,
    pub title: String,
    pub title_source: TitleSource,
    /// Live "doing now" line; empty when idle.
    pub activity: String,
    pub busy: bool,
    /// Free-form phase label ("idle" | "working" | distiller phases).
    pub phase: String,
    /// Monotonic per-session turn counter (0 before the first turn).
    pub turn_id: i64,
    pub turn_started_at: u64,
    pub last_distill_at: u64,
    /// Liveness clock: last heartbeat (Local) or last emit (Peer).
    pub last_seen: u64,
    /// Harness resume token (empty for peers / non-resumable).
    pub resume_id: String,
    /// Bumped on every public-content change (NOT on heartbeat). Pairs with the
    /// outbox row and the distill base-version guard.
    pub state_version: i64,
    pub lifecycle: Lifecycle,
    /// Set ONLY on insert; lets readers detect "appeared since X".
    pub first_seen: u64,
    /// Bumped in lockstep with `state_version`; lets readers detect "changed
    /// since X" without a heartbeat false-positive.
    pub updated_at: u64,
}

// ── derived status (the one shared projection) ───────────────────────────────

/// Liveness verdict from `last_seen` vs `now` against `STATUS_TTL_SECS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Live,
    Stale,
}

impl Liveness {
    pub fn is_live(&self) -> bool {
        matches!(self, Liveness::Live)
    }
}

/// The single derived view every reader renders. `busy` is the row's mid-turn
/// flag; `liveness` is freshness; `activity` is blanked unless busy so an idle
/// session never shows a stale "doing now" line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedStatus {
    pub busy: bool,
    pub liveness: Liveness,
    pub title: String,
    pub activity: String,
    pub lifecycle: Lifecycle,
    /// `now - last_seen` (saturating).
    pub age_secs: u64,
}

/// The one projection shared by the publisher, both `who` branches,
/// `rpc_statusline`, and the turn delta. Pure: liveness = `last_seen` within
/// `STATUS_TTL_SECS` of `now`; activity is suppressed when not busy; an ended or
/// superseded lifecycle is never reported live.
pub fn derive_status(snap: &SessionSnapshot, now: u64) -> DerivedStatus {
    let age_secs = now.saturating_sub(snap.last_seen);
    let fresh = age_secs <= STATUS_TTL_SECS;
    let active_lifecycle = matches!(snap.lifecycle, Lifecycle::Active);
    let liveness = if fresh && active_lifecycle {
        Liveness::Live
    } else {
        Liveness::Stale
    };
    DerivedStatus {
        busy: snap.busy && active_lifecycle,
        liveness,
        title: snap.title.clone(),
        activity: if snap.busy && active_lifecycle {
            snap.activity.clone()
        } else {
            String::new()
        },
        lifecycle: snap.lifecycle,
        age_secs,
    }
}

// ── delta projection (turn-start / turn-check) ───────────────────────────────

/// How a session changed relative to a reader's cursor. `status_delta_since`
/// classifies every in-scope row into exactly one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    /// `first_seen >= since` and still live → newly joined.
    Appeared,
    /// `updated_at >= since` (a versioned content change) and still live.
    Changed,
    /// Lifecycle ended/superseded since `since`, OR liveness expired within the
    /// window → render as "gone".
    Gone,
}

/// One delta row: the change classification plus the full snapshot and its
/// derived status, so the shared renderer needs no second lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDeltaItem {
    pub kind: DeltaKind,
    pub snapshot: SessionSnapshot,
    pub derived: DerivedStatus,
}

// ── transition command vocabulary ────────────────────────────────────────────

/// The closed set of mutations the canonical aggregate accepts. Each variant
/// corresponds 1:1 to a `Store` transition method and is the unit a
/// `outbox` row represents. Carried for tests/telemetry; the methods are
/// the executable form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// `register_or_reassert_session` minted/reattached the session.
    Register,
    /// `start_turn` opened turn `turn_id` (busy -> true).
    StartTurn { turn_id: i64 },
    /// `apply_distill_result` applied a distilled (title, activity).
    Distill { turn_id: i64, base_version: i64 },
    /// `heartbeat_session` refreshed liveness ONLY (no version bump, no outbox).
    Heartbeat,
    /// `end_turn` closed the turn (busy -> false; title retained).
    EndTurn,
    /// `end_session` finished the session (lifecycle -> ended; title retained).
    EndSession,
    /// `supersede_session` retired a session a newer one replaced.
    Supersede,
}

// ── pure identity resolution ─────────────────────────────────────────────────

/// A live session candidate `resolve_identity` may reattach to or supersede.
/// The registry hands the rows it found (same host/channel/agent, lifecycle
/// active) so the decision stays a pure function of data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveLocator {
    pub session_id: SessionId,
    pub harness_session_id: Option<String>,
    pub resume_id: Option<String>,
    pub pty_session: Option<String>,
    pub watch_pid: Option<i32>,
}

/// The identity decision for an observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityDecision {
    /// An alias already maps to this canonical id → reassert it.
    Existing(SessionId),
    /// A live session shares the harness id / resume token → same logical
    /// session restarted in place; reattach to it.
    Reattach(SessionId),
    /// A live session occupies the same PTY/pid but is a DIFFERENT logical
    /// session → supersede the old one and mint a fresh canonical id.
    Supersede { old: SessionId },
    /// No prior identity → mint a fresh canonical id.
    Mint,
}

/// Decide identity from an alias lookup + the live candidates on the same
/// (host, channel, agent). Precedence, highest first:
///   1. `alias_hit`            → Existing (a stored alias already names the id)
///   2. same harness id / resume token among live → Reattach (restart in place)
///   3. same PTY session / watch_pid among live    → Supersede (slot reused by a
///      new logical session)
///   4. otherwise                                  → Mint
///
/// Pure and total; callers wrap the result in one SQLite transaction.
pub fn resolve_identity(
    obs: &SessionObservation,
    alias_hit: Option<SessionId>,
    live: &[LiveLocator],
) -> IdentityDecision {
    if let Some(id) = alias_hit {
        return IdentityDecision::Existing(id);
    }

    let eq_some = |a: &Option<String>, b: &Option<String>| match (a, b) {
        (Some(x), Some(y)) => !x.is_empty() && x == y,
        _ => false,
    };

    // 2. Same harness-native id or resume token → genuine reattach.
    for c in live {
        if eq_some(&c.harness_session_id, &obs.harness_session_id)
            || eq_some(&c.resume_id, &obs.resume_id)
        {
            return IdentityDecision::Reattach(c.session_id.clone());
        }
    }

    // 3. Same hosted endpoint or watched pid, but no id/resume match → new session in the
    //    same slot; supersede the incumbent.
    for c in live {
        let same_endpoint = eq_some(&c.pty_session, &obs.pty_session);
        let same_pid = match (c.watch_pid, obs.watch_pid) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        };
        if same_endpoint || same_pid {
            return IdentityDecision::Supersede {
                old: c.session_id.clone(),
            };
        }
    }

    IdentityDecision::Mint
}

/// Where a newly-born session's fabric events (chat, presence, membership)
/// land — a per-session room it mints, or a group it was handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomDecision {
    /// Human-initiated session (no group override): mint a per-session NIP-29
    /// subgroup as a leaf under `parent` and route the session's fabric events
    /// into it.
    Mint { parent: String },
    /// Orchestration-spawned session (channel override present): the backend
    /// already scoped this agent to subgroup `group`; join it as a member, do
    /// not mint a child room.
    UseExisting { group: String },
}

/// Decide whether a session-birth mints its own per-session room.
///
/// The discriminator is the wire-level `TENEX_EDGE_CHANNEL` override the daemon
/// reads as `channel`, gated by the operator's `perSessionRooms` config:
///   - `Some(g)`    → orchestration-spawned (the backend added this agent to
///     subgroup `g`) → use `g` as-is; no child room. (Always, regardless of the
///     `per_session_rooms` flag.)
///   - `None`/empty, `per_session_rooms = true` → human-initiated (someone ran
///     `claude` / `tenex-edge launch` directly) → mint a per-session subgroup
///     under `work_root`.
///   - `None`/empty, `per_session_rooms = false` (default) → land in the bare
///     root channel (`work_root`); no child room is minted.
///
/// Pure and total.
pub fn decide_session_room(
    group: Option<&str>,
    work_root: &str,
    per_session_rooms: bool,
) -> RoomDecision {
    match group {
        Some(g) if !g.is_empty() => RoomDecision::UseExisting {
            group: g.to_string(),
        },
        _ if per_session_rooms => RoomDecision::Mint {
            parent: work_root.to_string(),
        },
        _ => RoomDecision::UseExisting {
            group: work_root.to_string(),
        },
    }
}

#[cfg(test)]
mod tests;
