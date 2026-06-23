//! Pure session-aggregate types + identity resolution + status derivation.
//!
//! This module is the *shape* of a session: the canonical id the daemon mints,
//! the identity inputs the hook observes, the complete public snapshot every
//! reader projects from, and the pure helpers that decide identity and derive
//! liveness. It names NO Nostr concepts (no kinds, tags, relays) and performs
//! NO I/O — `state.rs` owns the SQLite side, `domain.rs` owns the wire-facing
//! `Status`. Everything here is deterministic and table-testable.
//!
//! The single source of truth for a live local session is the `session_state`
//! row (see `state.rs`); `SessionSnapshot` is its in-memory projection and is
//! ALSO the projection of a `peer_session_state` row, so one `derive_status`
//! serves local and peer readers identically.

use crate::domain::Lifecycle;

// The canonical session id IS `util::SessionId`. The daemon mints it; harness
// ids / resume tokens / tmux panes become aliases that resolve to it. Re-exported
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
    Unknown,
}

impl Harness {
    pub fn as_str(&self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Codex => "codex",
            Harness::Opencode => "opencode",
            Harness::Unknown => "unknown",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "claude-code" | "claude" => Harness::ClaudeCode,
            "codex" => Harness::Codex,
            "opencode" => Harness::Opencode,
            _ => Harness::Unknown,
        }
    }
}

/// The kind of external identifier carried by a `session_aliases` row. Together
/// with `harness` + the raw `external_id` it forms the alias PK, so the same
/// pane id under two harnesses (or a resume token vs a harness-native id) never
/// collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AliasKind {
    /// The harness-native session id (claude/codex adopt it; opencode `ses_*`).
    HarnessSession,
    /// A `--resume` token distinct from the harness id (opencode).
    Resume,
    /// A tmux pane id (e.g. `%5`).
    TmuxPane,
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
            AliasKind::TmuxPane => "tmux_pane",
            AliasKind::WatchPid => "watch_pid",
            AliasKind::Generated => "generated",
        }
    }
}

/// Where a `title` came from. Higher-fidelity sources win: a `Distill` title is
/// never overwritten by a `Seed`, and `seed_title_if_empty` only acts when the
/// current source is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSource {
    None,
    /// Quick-seeded from the user prompt at turn start.
    Seed,
    /// Produced by the LLM distiller.
    Distill,
    /// Mirrored from a peer's kind:30315 wire title.
    Peer,
}

impl TitleSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            TitleSource::None => "none",
            TitleSource::Seed => "seed",
            TitleSource::Distill => "distill",
            TitleSource::Peer => "peer",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "seed" => TitleSource::Seed,
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
    pub project: String,
    pub agent_pubkey: String,
    pub agent_slug: String,
    pub harness: Harness,
    /// Harness-native session id, when the harness supplies one.
    pub harness_session_id: Option<String>,
    /// Resume token (claude/codex == harness id; opencode `ses_*`).
    pub resume_id: Option<String>,
    /// tmux pane id (e.g. `%5`), when running inside tmux.
    pub tmux_pane: Option<String>,
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
    pub project: String,
    pub host: String,
    pub rel_cwd: String,
    pub harness: Harness,
    pub harness_session_id: Option<String>,
    pub resume_id: Option<String>,
    pub tmux_pane: Option<String>,
    pub watch_pid: Option<i32>,
    /// Wall-clock seconds the hook fired; becomes `last_seen`/`first_seen`.
    pub observed_at: u64,
}

impl SessionObservation {
    /// Project the identity inputs out of the observation.
    pub fn key(&self) -> SessionKey {
        SessionKey {
            host: self.host.clone(),
            project: self.project.clone(),
            agent_pubkey: self.agent_pubkey.clone(),
            agent_slug: self.agent_slug.clone(),
            harness: self.harness,
            harness_session_id: self.harness_session_id.clone(),
            resume_id: self.resume_id.clone(),
            tmux_pane: self.tmux_pane.clone(),
            watch_pid: self.watch_pid.clone(),
        }
    }
}

/// What `Store::record_peer_status` receives from the kind:30315 materializer.
/// Mirrors a peer's self-contained per-session signal into `peer_session_state`.
/// Keyed by `(agent_pubkey, project)` — one row per agent per group, no session_id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerStatusObservation {
    pub agent_pubkey: String,
    /// Resolved from the peer's kind:0 profile (NEVER self-asserted); may be "".
    pub agent_slug: String,
    /// Group id (== kind:30315 `d` tag == `h` tag == project slug).
    pub project: String,
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

/// Whether a snapshot came from this machine's authoritative `session_state` or
/// the materialized `peer_session_state` mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotSource {
    Local,
    Peer,
}

/// The complete public state of ONE session — the unified projection of both a
/// `session_state` row (Local) and a `peer_session_state` row (Peer). Every
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
    pub project: String,
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
    /// status_outbox PK and the distill base-version guard.
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
/// `status_outbox` row represents. Carried for tests/telemetry; the methods are
/// the executable form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// `register_or_reassert_session` minted/reattached the session.
    Register,
    /// `start_turn` opened turn `turn_id` (busy -> true).
    StartTurn { turn_id: i64 },
    /// `seed_title_if_empty` placed a provisional title.
    SeedTitle,
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
/// The registry hands the rows it found (same host/project/agent, lifecycle
/// active) so the decision stays a pure function of data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveLocator {
    pub session_id: SessionId,
    pub harness_session_id: Option<String>,
    pub resume_id: Option<String>,
    pub tmux_pane: Option<String>,
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
    /// A live session occupies the same pane/pid but is a DIFFERENT logical
    /// session → supersede the old one and mint a fresh canonical id.
    Supersede { old: SessionId },
    /// No prior identity → mint a fresh canonical id.
    Mint,
}

/// Decide identity from an alias lookup + the live candidates on the same
/// (host, project, agent). Precedence, highest first:
///   1. `alias_hit`            → Existing (a stored alias already names the id)
///   2. same harness id / resume token among live → Reattach (restart in place)
///   3. same tmux pane / watch_pid among live      → Supersede (slot reused by a
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

    // 3. Same pane or watched pid, but no id/resume match → new session in the
    //    same slot; supersede the incumbent.
    for c in live {
        let same_pane = eq_some(&c.tmux_pane, &obs.tmux_pane);
        let same_pid = match (c.watch_pid, obs.watch_pid) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        };
        if same_pane || same_pid {
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
/// reads as `channel`:
///   - `None`/empty → human-initiated (someone ran `claude` / `tenex-edge
///     launch` directly) → mint a per-session subgroup under `work_root`.
///   - `Some(g)`    → orchestration-spawned (the backend added this agent to
///     subgroup `g`) → use `g` as-is; no child room.
///
/// Pure and total.
pub fn decide_session_room(group: Option<&str>, work_root: &str) -> RoomDecision {
    match group {
        Some(g) if !g.is_empty() => RoomDecision::UseExisting { group: g.to_string() },
        _ => RoomDecision::Mint {
            parent: work_root.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs() -> SessionObservation {
        SessionObservation {
            agent_slug: "claude".into(),
            agent_pubkey: "pk".into(),
            project: "proj".into(),
            host: "laptop".into(),
            rel_cwd: String::new(),
            harness: Harness::ClaudeCode,
            harness_session_id: None,
            resume_id: None,
            tmux_pane: None,
            watch_pid: None,
            observed_at: 100,
        }
    }

    fn snap(busy: bool, last_seen: u64, lifecycle: Lifecycle) -> SessionSnapshot {
        SessionSnapshot {
            source: SnapshotSource::Local,
            session_id: SessionId::from("s1"),
            agent_pubkey: "pk".into(),
            agent_slug: "claude".into(),
            project: "proj".into(),
            host: "laptop".into(),
            rel_cwd: String::new(),
            title: "fixing auth".into(),
            title_source: TitleSource::Distill,
            activity: "editing handler".into(),
            busy,
            phase: "working".into(),
            turn_id: 1,
            turn_started_at: 0,
            last_distill_at: 0,
            last_seen,
            resume_id: String::new(),
            state_version: 3,
            lifecycle,
            first_seen: 50,
            updated_at: 90,
        }
    }

    #[test]
    fn alias_hit_is_existing() {
        let d = resolve_identity(&obs(), Some(SessionId::from("canon")), &[]);
        assert_eq!(d, IdentityDecision::Existing(SessionId::from("canon")));
    }

    #[test]
    fn resume_match_reattaches() {
        let mut o = obs();
        o.resume_id = Some("ses_x".into());
        let live = vec![LiveLocator {
            session_id: SessionId::from("canon"),
            harness_session_id: None,
            resume_id: Some("ses_x".into()),
            tmux_pane: Some("%5".into()),
            watch_pid: Some(10),
        }];
        assert_eq!(
            resolve_identity(&o, None, &live),
            IdentityDecision::Reattach(SessionId::from("canon"))
        );
    }

    #[test]
    fn same_pane_different_session_supersedes() {
        let mut o = obs();
        o.tmux_pane = Some("%5".into());
        o.harness_session_id = Some("new".into());
        let live = vec![LiveLocator {
            session_id: SessionId::from("old"),
            harness_session_id: Some("old".into()),
            resume_id: None,
            tmux_pane: Some("%5".into()),
            watch_pid: None,
        }];
        assert_eq!(
            resolve_identity(&o, None, &live),
            IdentityDecision::Supersede {
                old: SessionId::from("old")
            }
        );
    }

    #[test]
    fn no_signal_mints() {
        assert_eq!(resolve_identity(&obs(), None, &[]), IdentityDecision::Mint);
    }

    #[test]
    fn derive_live_busy() {
        let d = derive_status(&snap(true, 1000, Lifecycle::Active), 1000);
        assert!(d.liveness.is_live());
        assert!(d.busy);
        assert_eq!(d.activity, "editing handler");
    }

    #[test]
    fn derive_idle_blanks_activity() {
        let d = derive_status(&snap(false, 1000, Lifecycle::Active), 1000);
        assert!(!d.busy);
        assert_eq!(d.activity, "");
        assert_eq!(d.title, "fixing auth");
    }

    #[test]
    fn derive_stale_when_past_ttl() {
        let d = derive_status(&snap(true, 0, Lifecycle::Active), STATUS_TTL_SECS + 1);
        assert_eq!(d.liveness, Liveness::Stale);
    }

    #[test]
    fn derive_ended_is_never_live() {
        let d = derive_status(&snap(true, 1000, Lifecycle::Ended), 1000);
        assert_eq!(d.liveness, Liveness::Stale);
        assert!(!d.busy);
    }

    // ── per-session room minting decision ───────────────────────────────

    #[test]
    fn room_no_group_override_mints_under_work_root() {
        // Human-initiated: someone ran `claude` / `tenex-edge launch` directly,
        // so TENEX_EDGE_CHANNEL is unset → mint a per-session room under the repo.
        assert_eq!(
            decide_session_room(None, "my-repo"),
            RoomDecision::Mint {
                parent: "my-repo".into()
            }
        );
    }

    #[test]
    fn room_empty_group_override_mints_under_work_root() {
        // An empty override is treated identically to absent (the daemon reads
        // TENEX_EDGE_CHANNEL="" as "no channel").
        assert_eq!(
            decide_session_room(Some(""), "my-repo"),
            RoomDecision::Mint {
                parent: "my-repo".into()
            }
        );
    }

    #[test]
    fn room_with_group_override_uses_existing() {
        // Orchestration-spawned: the backend added this agent to subgroup
        // `issue-514` → join it, do not mint a child room.
        assert_eq!(
            decide_session_room(Some("issue-514"), "my-repo"),
            RoomDecision::UseExisting {
                group: "issue-514".into()
            }
        );
    }
}
