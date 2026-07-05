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
        mode: SurfaceMode::Authoritative,
        facts: &["hook call", "cursor", "store snapshot", "now"],
        trellis_inputs: &["ViewInputs"],
        host_effects: &["materialized FabricView output text"],
        bypass_risks: &[],
    },
    SurfaceRegistration {
        name: "turn_lifecycle",
        mode: SurfaceMode::Authoritative,
        facts: &[
            "InputFact::TurnStarted",
            "InputFact::TurnEnded",
            "InputFact::TranscriptWindowCaptured",
        ],
        trellis_inputs: &[
            "InputFact::TurnStarted",
            "InputFact::TurnEnded",
            "InputFact::TranscriptWindowCaptured",
        ],
        host_effects: &["turn_lifecycle executor applies sessions projection"],
        bypass_risks: &[],
    },
    SurfaceRegistration {
        name: "cursor",
        mode: SurfaceMode::Authoritative,
        facts: &["render cursor observation"],
        trellis_inputs: &["InputFact::TurnCheckRequested"],
        host_effects: &["cursor executor applies sessions.seen_cursor"],
        bypass_risks: &[],
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
        mode: SurfaceMode::Authoritative,
        facts: &[
            "InputFact::OutboxEnqueueApplied",
            "RelayPublishAccepted",
            "RelayPublishFailed",
        ],
        trellis_inputs: &[
            "InputFact::OutboxEnqueueApplied",
            "InputFact::RelayPublishAccepted",
        ],
        host_effects: &["outbox seam applies durable queue projection"],
        bypass_risks: &[],
    },
];
