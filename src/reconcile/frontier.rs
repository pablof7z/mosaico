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
        // Advisory is the intentional ceiling for this surface (#228 §8): Trellis
        // can derive staged intents, but cannot prove DB/relay/spawn/tmux effects.
        mode: SurfaceMode::Advisory,
        facts: &[
            "InputFact::SessionStartRequested",
            "InputFact::SessionStarted",
            "InputFact::SessionStartFailed",
        ],
        trellis_inputs: &[
            "InputFact::SessionStartRequested",
            "InputFact::SessionStarted",
            "InputFact::SessionStartFailed",
        ],
        host_effects: &[
            "rpc_session_start executes advisory staged intents",
            "session_start advisory records request/outcome facts",
        ],
        bypass_risks: &[],
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
