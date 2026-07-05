//! Code-owned authority frontier for Trellis adoption.
//!
//! This is the single source of truth for surface mode assignments. Probe output,
//! oracle honesty, and bypass ratchets read this registry instead of duplicating
//! the design table in docs.

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceMode {
    Imperative,
    Shadow,
    Advisory,
    Authoritative,
    ProjectionOwned,
}

impl SurfaceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SurfaceMode::Imperative => "imperative",
            SurfaceMode::Shadow => "shadow",
            SurfaceMode::Advisory => "advisory",
            SurfaceMode::Authoritative => "authoritative",
            SurfaceMode::ProjectionOwned => "projection-owned",
        }
    }

    pub fn is_authoritative_plus(self) -> bool {
        matches!(
            self,
            SurfaceMode::Authoritative | SurfaceMode::ProjectionOwned
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceRegistration {
    pub name: &'static str,
    pub mode: SurfaceMode,
    pub facts: &'static [&'static str],
    pub trellis_inputs: &'static [&'static str],
    pub host_effects: &'static [&'static str],
    pub bypass_risks: &'static [&'static str],
}

pub fn registrations() -> &'static [SurfaceRegistration] {
    &REGISTRATIONS
}

pub fn host_seam_coverage_percent() -> i64 {
    let covered = REGISTRATIONS
        .iter()
        .filter(|r| r.mode.is_authoritative_plus())
        .count();
    ((covered * 100) / REGISTRATIONS.len()) as i64
}

pub fn uncovered_bypass_risks() -> Vec<&'static str> {
    REGISTRATIONS
        .iter()
        .filter(|r| !r.mode.is_authoritative_plus())
        .flat_map(|r| r.bypass_risks.iter().copied())
        .collect()
}

static REGISTRATIONS: [SurfaceRegistration; 7] = [
    SurfaceRegistration {
        name: "status",
        mode: SurfaceMode::Authoritative,
        facts: &[
            "session lifecycle",
            "turn lifecycle",
            "distill result",
            "heartbeat tick",
            "channel membership",
        ],
        trellis_inputs: &[
            "session-local",
            "session-identity",
            "session-channel-set",
            "now",
        ],
        host_effects: &["status_seam::drive enqueues signed kind:30315 events"],
        bypass_risks: &["direct kind:30315 publish outside status_seam"],
    },
    SurfaceRegistration {
        name: "subscriptions",
        mode: SurfaceMode::Authoritative,
        facts: &[
            "daemon channel pins",
            "alive sessions",
            "memberships",
            "local pubkeys",
        ],
        trellis_inputs: &["CoverageSnapshot"],
        host_effects: &["daemon/server/subscriptions.rs applies Open/Close/Replace"],
        bypass_risks: &["direct relay subscribe/unsubscribe outside subscription executor"],
    },
    SurfaceRegistration {
        name: "hook_context",
        mode: SurfaceMode::Advisory,
        facts: &["hook call", "cursor", "store snapshot", "now"],
        trellis_inputs: &["ViewInputs"],
        host_effects: &["materialized FabricView output text"],
        bypass_risks: &[],
    },
    SurfaceRegistration {
        name: "turn_lifecycle",
        mode: SurfaceMode::Imperative,
        facts: &["turn_start", "turn_end", "transcript append"],
        trellis_inputs: &["InputFact::TurnStarted", "InputFact::TurnEnded"],
        host_effects: &["sessions.working", "turn transcript rows"],
        bypass_risks: &["rpc_turn_start"],
    },
    SurfaceRegistration {
        name: "cursor",
        mode: SurfaceMode::Imperative,
        facts: &["post-tool cursor observation"],
        trellis_inputs: &["InputFact::CursorAdvanced"],
        host_effects: &["sessions.seen_cursor"],
        bypass_risks: &["cursor CAS"],
    },
    SurfaceRegistration {
        name: "session_start",
        mode: SurfaceMode::Imperative,
        facts: &[
            "session_start RPC",
            "signer choice",
            "relay readiness",
            "tmux spawn",
        ],
        trellis_inputs: &["InputFact::SessionStarted"],
        host_effects: &[
            "session row",
            "identity row",
            "relay membership",
            "tmux pane",
        ],
        bypass_risks: &["rpc_session_start"],
    },
    SurfaceRegistration {
        name: "outbox",
        mode: SurfaceMode::Imperative,
        facts: &[
            "pending signed event",
            "RelayPublishAccepted",
            "RelayPublishFailed",
        ],
        trellis_inputs: &["InputFact::RelayPublishAccepted"],
        host_effects: &["relay publish", "outbox row state"],
        bypass_risks: &["outbox publish"],
    },
];
