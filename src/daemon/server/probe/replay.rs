//! `probe replay <capsule>` (§4.4): load a stored replay capsule and, on
//! request, assert a separate-process-style replay or export a Flight Recorder
//! `SerializedScenario` trace.

use super::{required_str, DaemonState};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) fn replay_value(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let capsule = required_str(params, "capsule")?;
    let id: i64 = capsule
        .parse()
        .with_context(|| format!("probe replay: capsule id must be an integer, got `{capsule}`"))?;
    let assert = params
        .get("assert")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let export_trace = params
        .get("export_trace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let row = state
        .with_store(|s| s.get_replay_capsule(id))?
        .with_context(|| format!("probe replay: capsule {id} not found"))?;

    let report = if assert || export_trace {
        Some(crate::reconcile::replay::replay_script_json(
            &row.script_json,
            export_trace,
        )?)
    } else {
        None
    };

    Ok(json!({
        "verb": "replay",
        "capsule": {
            "id": row.id,
            "surface": row.surface,
            "trigger_kind": row.trigger_kind,
            "trigger_ref": row.trigger_ref,
            "script_bytes": row.script_bytes,
            "format_version": row.format_version,
            "created_at": row.created_at,
        },
        "asserted": report.is_some(),
        "ok": true,
        "steps": report.as_ref().map(|r| r.steps).unwrap_or(0),
        "resource_commands": report.as_ref().map(|r| r.resource_commands).unwrap_or(0),
        "output_frames": report.as_ref().map(|r| r.output_frames).unwrap_or(0),
        "trace_json": report.and_then(|r| r.trace_json),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::{InputFact, StatusDrive};
    use trellis_testing::DataTransactionScript;

    #[tokio::test]
    async fn replays_stored_capsule() {
        let state = DaemonState::new_for_test().await;
        let mut script = DataTransactionScript::new();
        script
            .step("tick")
            .operation(InputFact::StatusDrive(StatusDrive::Tick {
                session_id: "missing".into(),
                at: 100,
            }))
            .commit();
        let script_json = script.to_json().unwrap();
        let id = state
            .with_store(|s| {
                s.record_replay_capsule(&crate::state::trellis_replay_capsules::NewReplayCapsule {
                    surface: "status".into(),
                    trigger_kind: "tick".into(),
                    trigger_ref: "missing".into(),
                    script_json,
                    format_version: 1,
                    created_at: 1000,
                })
            })
            .unwrap();

        let value = replay_value(
            &state,
            &json!({ "verb": "replay", "capsule": id.to_string(), "assert": true }),
        )
        .unwrap();
        assert_eq!(value["asserted"], true);
        assert_eq!(value["steps"], 1);
    }
}
