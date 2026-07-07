//! Shared InputFact classification for probe commands.
//!
//! A well-formed fact should always produce an explanation. Facts without a
//! daemon Trellis surface are reported as not proven instead of being collapsed
//! into a generic unsupported error.

use crate::reconcile::{InputFact, StatusDrive};
use serde_json::{json, Value};

pub(super) fn fact_evidence(fact: &InputFact, surface: Option<&str>) -> Value {
    match surface {
        Some(surface) => json!({
            "kind": fact_kind(fact),
            "at": fact.at(),
            "supported": true,
            "surface": surface,
            "summary": format!("{} is covered by the {surface} surface", fact_kind(fact)),
        }),
        None => unsupported_fact(fact),
    }
}

pub(super) fn unsupported_simulation(fact: &InputFact) -> Value {
    json!({
        "verb": "simulate",
        "surface": Value::Null,
        "fact": serde_json::to_value(fact).unwrap_or(Value::Null),
        "commands": [],
        "changed": [],
        "output_frames": 0,
        "would_effect": false,
        "would_publish": false,
        "simulated": false,
        "ok": false,
        "fact_evidence": unsupported_fact(fact),
    })
}

fn unsupported_fact(fact: &InputFact) -> Value {
    let kind = fact_kind(fact);
    let (frontier, reason) = match fact {
        InputFact::RelayEventObserved { .. } => (
            "event_ingest",
            "relay event ingestion still writes state::events imperatively; no Trellis surface owns insert_event",
        ),
        InputFact::ProcessExited { .. } => (
            "session_liveness",
            "process liveness is only modeled by the generic proof-of-life spine; no daemon surface owns mark_dead",
        ),
        InputFact::ClockTick { .. } => (
            "timekeeping",
            "clock ticks still feed several imperative loops; no single Trellis surface owns this fact",
        ),
        _ => (
            "unknown",
            "this fact is not registered with a validation surface",
        ),
    };
    json!({
        "kind": kind,
        "at": fact.at(),
        "supported": false,
        "frontier": frontier,
        "summary": format!("{kind} has no validating Trellis surface yet"),
        "reason": reason,
    })
}

fn fact_kind(fact: &InputFact) -> &'static str {
    match fact {
        InputFact::SessionStartRequested(_) => "SessionStartRequested",
        InputFact::StatusDrive(drive) => status_drive_kind(drive),
        InputFact::SubscriptionSync { .. } => "SubscriptionSync",
        InputFact::HookContextRender(_) => "HookContextRender",
        InputFact::DeliveryScan(_) => "DeliveryScan",
        InputFact::TurnCheckRequested { .. } => "TurnCheckRequested",
        InputFact::SessionStarted { .. } => "SessionStarted",
        InputFact::TurnStarted { .. } => "TurnStarted",
        InputFact::TranscriptWindowCaptured { .. } => "TranscriptWindowCaptured",
        InputFact::DistillCompleted { .. } => "DistillCompleted",
        InputFact::TurnEnded { .. } => "TurnEnded",
        InputFact::RelayEventObserved { .. } => "RelayEventObserved",
        InputFact::OutboxEnqueueApplied { .. } => "OutboxEnqueueApplied",
        InputFact::RelayPublishAccepted { .. } => "RelayPublishAccepted",
        InputFact::SessionStartFailed(_) => "SessionStartFailed",
        InputFact::ProcessExited { .. } => "ProcessExited",
        InputFact::ClockTick { .. } => "ClockTick",
    }
}

fn status_drive_kind(drive: &StatusDrive) -> &'static str {
    match drive {
        StatusDrive::SessionStarted(_) => "StatusDrive::SessionStarted",
        StatusDrive::TurnStarted { .. } => "StatusDrive::TurnStarted",
        StatusDrive::TurnEnded { .. } => "StatusDrive::TurnEnded",
        StatusDrive::DistillCompleted { .. } => "StatusDrive::DistillCompleted",
        StatusDrive::ChannelsChanged { .. } => "StatusDrive::ChannelsChanged",
        StatusDrive::Tick { .. } => "StatusDrive::Tick",
        StatusDrive::SessionEnded { .. } => "StatusDrive::SessionEnded",
    }
}
