//! The canonical **input-journal**: facts the world hands the reconciler.
//!
//! Each [`InputFact`] is an observation the host made (a session started, a
//! turn began, a relay accepted a publish, a process exited). The reconciler
//! folds these into Trellis inputs; the graph then *derives* all reconciled
//! state from them. The graph must never invent one of these facts — it may
//! only decide what to do about the ones it is given.
//!
//! Fields are the MINIMAL identifying + summary data a reconciler needs. Bulky
//! payloads stay out: a captured transcript window enters as a `window_hash`
//! (a stable content pointer), and a distillation enters as its short `title`
//! and `activity` strings — never the transcript text or raw event body.
//!
//! Each variant's doc comment names the real tenex-edge writer it will
//! eventually REPLACE, so the surface-implementation agents know exactly which
//! bespoke `Store` call becomes a fact-plus-plan pair.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::subscriptions::CoverageSnapshot;

/// A monotonic host timestamp (unix seconds), as tenex-edge already uses for
/// `enqueued_at`, `turn_started_at`, `last_seen`, etc.
pub type Timestamp = u64;

/// The status-surface drive inputs needed to replay one committed transaction.
///
/// These are intentionally the existing `StatusReconciler` entrypoints captured
/// as data. They are not durable product doctrine; later frontier slices can
/// replace them with the smaller canonical lifecycle/distill journal facts once
/// those facts become the first writer for the status surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusDrive {
    SessionStarted(StatusSessionStartedArgs),
    TurnStarted {
        session_id: String,
        at: Timestamp,
    },
    TurnEnded {
        session_id: String,
        at: Timestamp,
    },
    DistillCompleted {
        session_id: String,
        title: String,
        activity: String,
        window_hash: Option<String>,
        at: Timestamp,
    },
    ChannelsChanged {
        session_id: String,
        channels: BTreeSet<String>,
        at: Timestamp,
    },
    Tick {
        session_id: String,
        at: Timestamp,
    },
    SessionEnded {
        session_id: String,
        at: Timestamp,
    },
}

impl StatusDrive {
    pub fn at(&self) -> Timestamp {
        match self {
            Self::SessionStarted(args) => args.at,
            Self::TurnStarted { at, .. }
            | Self::TurnEnded { at, .. }
            | Self::DistillCompleted { at, .. }
            | Self::ChannelsChanged { at, .. }
            | Self::Tick { at, .. }
            | Self::SessionEnded { at, .. } => *at,
        }
    }
}

/// Arguments for `StatusReconciler::on_session_started`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSessionStartedArgs {
    pub session_id: String,
    pub host: String,
    pub slug: String,
    pub pubkey: String,
    pub rel_cwd: String,
    pub channels: BTreeSet<String>,
    pub working: bool,
    pub title: String,
    pub activity: String,
    pub at: Timestamp,
}

/// A frozen hook-context render input. `inputs_json` is the serde form of
/// `fabric_context::ViewInputs`; it remains JSON here so the public `InputFact`
/// type does not expose that private render-capture structure as API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookContextRenderFact {
    pub session_id: String,
    pub hook_kind: String,
    pub cursor: i64,
    pub now: i64,
    pub force: bool,
    pub emitted_text_hash: Option<String>,
    pub inputs_json: serde_json::Value,
}

/// One canonical world-fact, appended to the input journal by the host.
///
/// Derive set matches the daemon's data conventions: `Clone`/`Debug` for
/// plumbing and `serde` so a journal can be persisted, replayed, or diffed in
/// tests against the Trellis oracle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputFact {
    /// Transitional replay fact for the current authoritative status surface.
    StatusDrive(StatusDrive),

    /// Transitional replay fact for the current authoritative subscriptions
    /// surface: one complete coverage snapshot handed to the reconciler.
    SubscriptionSync {
        snapshot: CoverageSnapshot,
        at: Timestamp,
    },

    /// Transitional replay fact for the current hook-context render surface.
    HookContextRender(HookContextRenderFact),

    /// A fabric render asked whether it should advance the session cursor.
    ///
    /// Replaces the daemon's independent `seen_cursor` compare-and-swap decision.
    TurnCheckRequested {
        /// Session whose hook requested a check.
        session_id: String,
        /// The cursor observed on the session row before asking Trellis.
        observed_cursor: Timestamp,
        /// Whether this render is allowed to advance the cursor.
        working: bool,
        /// Current host time.
        at: Timestamp,
    },

    /// A new agent session became known to the daemon.
    ///
    /// Replaces `state::sessions::Store::register_session` /
    /// `upsert_session_row` writing the initial `sessions` row.
    SessionStarted {
        /// Canonical session id.
        session_id: String,
        /// Channel hash the session is bound to, if any.
        channel_h: Option<String>,
        /// Agent pubkey owning the session, if known at start.
        agent_pubkey: Option<String>,
        /// Host pid to watch for liveness, if the session is process-backed.
        pid: Option<i32>,
        /// When the session started.
        at: Timestamp,
    },

    /// A turn began for a session (the agent is now working).
    ///
    /// Replaces `state::sessions::Store::set_working(id, working = true,
    /// turn_started_at)`.
    TurnStarted {
        /// Session whose turn started.
        session_id: String,
        /// When the turn started.
        at: Timestamp,
    },

    /// A transcript window was captured for a session's current turn.
    ///
    /// Carries only the stable content `window_hash` — never the transcript
    /// text. Replaces `state::sessions::Store::set_session_transcript` writing
    /// the transcript pointer.
    TranscriptWindowCaptured {
        /// Session the window belongs to.
        session_id: String,
        /// Stable hash of the captured transcript window (the pointer, not the
        /// body).
        window_hash: String,
        /// When the window was captured.
        at: Timestamp,
    },

    /// A distillation completed for a previously captured transcript window.
    ///
    /// Carries only the distilled `title`/`activity` summary strings.
    /// Replaces `state::sessions::Store::set_session_distill(id, title,
    /// activity, last_distill_at)`.
    DistillCompleted {
        /// Session that was distilled.
        session_id: String,
        /// The transcript window this distillation summarizes.
        window_hash: String,
        /// Distilled human-readable title.
        title: String,
        /// Distilled current-activity summary.
        activity: String,
        /// When the distillation completed.
        at: Timestamp,
    },

    /// A turn ended for a session (the agent stopped working).
    ///
    /// Replaces `state::sessions::Store::set_working(id, working = false, ..)`.
    TurnEnded {
        /// Session whose turn ended.
        session_id: String,
        /// When the turn ended.
        at: Timestamp,
    },

    /// A relay event was observed (received) by the daemon.
    ///
    /// Carries only identifying header fields, never the event body. Replaces
    /// `state::events::Store::insert_event` recording the observed event.
    RelayEventObserved {
        /// Event id.
        event_id: String,
        /// Nostr event kind.
        kind: u32,
        /// Channel hash the event targets, if any.
        channel_h: Option<String>,
        /// Author pubkey.
        pubkey: String,
        /// When the event was observed.
        at: Timestamp,
    },

    /// A graph-derived publish intent was signed and inserted into `outbox`.
    OutboxEnqueueApplied {
        local_id: i64,
        event_id: String,
        event_hash: String,
        source_surface: String,
        source_ref: String,
        at: Timestamp,
    },

    /// A relay reported the outcome of an outbound publish.
    RelayPublishAccepted {
        local_id: i64,
        event_id: String,
        accepted: bool,
        error: Option<String>,
        at: Timestamp,
    },

    /// A watched host process exited.
    ///
    /// Replaces the `runtime` pid watcher (`pid_alive`) that calls
    /// `state::sessions::Store::mark_dead` when the host pid disappears.
    ProcessExited {
        /// Session backed by the process, if the pid maps to one.
        session_id: Option<String>,
        /// The pid that exited.
        pid: i32,
        /// When the exit was observed.
        at: Timestamp,
    },

    /// A monotonic clock tick, so the graph can reconcile time-based decisions
    /// (status expiry, retention) from an explicit input rather than reading a
    /// clock during propagation.
    ///
    /// Replaces the ad-hoc `now`/heartbeat reads scattered through the
    /// `runtime` loop (e.g. `touch_session`, status expiry).
    ClockTick {
        /// Current host time.
        at: Timestamp,
    },
}

impl InputFact {
    /// The host timestamp carried by this fact.
    pub fn at(&self) -> Timestamp {
        match self {
            Self::StatusDrive(drive) => drive.at(),
            Self::SubscriptionSync { at, .. } => *at,
            Self::HookContextRender(fact) => fact.now.max(0) as Timestamp,
            Self::TurnCheckRequested { at, .. } => *at,
            Self::SessionStarted { at, .. }
            | Self::TurnStarted { at, .. }
            | Self::TranscriptWindowCaptured { at, .. }
            | Self::DistillCompleted { at, .. }
            | Self::TurnEnded { at, .. }
            | Self::RelayEventObserved { at, .. }
            | Self::OutboxEnqueueApplied { at, .. }
            | Self::RelayPublishAccepted { at, .. }
            | Self::ProcessExited { at, .. }
            | Self::ClockTick { at } => *at,
        }
    }
}
