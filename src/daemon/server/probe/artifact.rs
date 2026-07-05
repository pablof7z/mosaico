use super::DaemonState;
use crate::reconcile::journal::{InputFact, StatusDrive};
use crate::reconcile::labels::{key_path, NodeLabels};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use trellis_core::{ResourceCommand, TransactionResult};
use trellis_testing::DataTransactionScript;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Artifact {
    pub surface: &'static str,
    pub value: Value,
    pub hash: String,
}

pub(super) fn fact_param(params: &Value, key: &str) -> Result<Option<InputFact>> {
    let Some(raw) = params.get(key) else {
        return Ok(None);
    };
    let fact = match raw {
        Value::String(s) => serde_json::from_str(s).context("probe: invalid fact JSON")?,
        value => serde_json::from_value(value.clone()).context("probe: invalid fact")?,
    };
    Ok(Some(fact))
}

pub(super) fn infer_surface(fact: &InputFact) -> Option<&'static str> {
    match fact {
        InputFact::StatusDrive(_) | InputFact::DistillCompleted { .. } => Some("status"),
        InputFact::TurnStarted { .. }
        | InputFact::TurnEnded { .. }
        | InputFact::TranscriptWindowCaptured { .. } => Some("turn_lifecycle"),
        InputFact::TurnCheckRequested { .. } => Some("cursor"),
        InputFact::OutboxEnqueueApplied { .. } | InputFact::RelayPublishAccepted { .. } => {
            Some("outbox")
        }
        InputFact::SubscriptionSync { .. } => Some("subscriptions"),
        _ => None,
    }
}

pub(super) fn preview_artifact(state: &Arc<DaemonState>, fact: &InputFact) -> Result<Artifact> {
    match infer_surface(fact).context("probe: unsupported InputFact")? {
        "status" => preview_status(state, &normalize_status_fact(fact.clone())?),
        "turn_lifecycle" => preview_turn_lifecycle(state, fact),
        "cursor" => preview_cursor(state, fact),
        "outbox" => preview_outbox(state, fact),
        "subscriptions" => preview_subscriptions(state, fact),
        _ => unreachable!("surface inferred above"),
    }
}

pub(super) fn empty_artifact(surface: &'static str) -> Artifact {
    hashed(
        surface,
        json!({
            "commands": [],
            "changed": [],
            "output_frames": [],
        }),
    )
}

fn preview_status(state: &Arc<DaemonState>, fact: &InputFact) -> Result<Artifact> {
    let mut r = state.status.lock().expect("status mutex poisoned");
    let preview = r
        .preview_fact(fact)
        .map_err(|e| anyhow::anyhow!("status preview failed: {e:?}"))?
        .context("probe: fact is not supported by status")?;
    Ok(plan_artifact(
        "status",
        &preview.labels,
        &preview.result,
        Some(30315),
    ))
}

fn preview_subscriptions(state: &Arc<DaemonState>, fact: &InputFact) -> Result<Artifact> {
    let mut r = state.subs.lock().expect("subscriptions mutex poisoned");
    let preview = r
        .preview_fact(fact)
        .map_err(|e| anyhow::anyhow!("subscription preview failed: {e:?}"))?
        .context("probe: fact is not supported by subscriptions")?;
    Ok(plan_artifact(
        "subscriptions",
        &preview.labels,
        &preview.result,
        None,
    ))
}

fn preview_turn_lifecycle(state: &Arc<DaemonState>, fact: &InputFact) -> Result<Artifact> {
    let mut r = state
        .turn_lifecycle
        .lock()
        .expect("turn lifecycle mutex poisoned");
    let preview = r
        .preview_fact(fact)
        .map_err(|e| anyhow::anyhow!("turn lifecycle preview failed: {e:?}"))?
        .context("probe: fact is not supported by turn_lifecycle")?;
    Ok(plan_artifact(
        "turn_lifecycle",
        &preview.labels,
        &preview.result,
        None,
    ))
}

fn preview_cursor(state: &Arc<DaemonState>, fact: &InputFact) -> Result<Artifact> {
    super::cursor_artifact::preview_cursor(state, fact)
}

fn preview_outbox(state: &Arc<DaemonState>, fact: &InputFact) -> Result<Artifact> {
    super::outbox_artifact::preview_outbox(state, fact)
}

fn plan_artifact<C>(
    surface: &'static str,
    labels: &NodeLabels,
    plan: &TransactionResult<C>,
    wire_kind: Option<u64>,
) -> Artifact {
    hashed(
        surface,
        json!({
            "commands": command_values(plan.resource_plan.commands(), wire_kind),
            "changed": labels.labels_for(&plan.changed_inputs),
            "output_frames": plan.output_frames.len(),
        }),
    )
}

pub(super) fn replay_artifact(script: &DataTransactionScript<InputFact>) -> Result<Artifact> {
    let report = crate::reconcile::replay::replay_script(script, true)?;
    let trace = report
        .trace_json
        .context("replay trace was not exported for artifact diff")?;
    let trace: Value = serde_json::from_str(&trace).context("decoding replay trace")?;
    let steps = trace
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let commands = flatten_trace_field(&steps, "resource_commands");
    let frames = flatten_trace_field(&steps, "output_frames");
    let surface = match report.surface.as_str() {
        "status" => "status",
        "subscriptions" => "subscriptions",
        "hook_context" => "hook_context",
        "turn_lifecycle" => "turn_lifecycle",
        "cursor" => "cursor",
        "outbox" => "outbox",
        _ => "unknown",
    };
    Ok(hashed(
        surface,
        json!({
            "commands": commands,
            "output_frames": frames,
            "steps": steps.len(),
        }),
    ))
}

fn flatten_trace_field(steps: &[Value], field: &str) -> Vec<Value> {
    steps
        .iter()
        .flat_map(|step| {
            step.get("trace")
                .and_then(|trace| trace.get(field))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .collect()
}

pub(super) fn replace_last_operation(
    script: &DataTransactionScript<InputFact>,
    fact: InputFact,
) -> Result<DataTransactionScript<InputFact>> {
    let mut target = None;
    for (step_idx, step) in script.steps().iter().enumerate() {
        if !step.operations().is_empty() {
            target = Some((step_idx, step.operations().len() - 1));
        }
    }
    let target = target.context("probe diff: capsule has no operations to mutate")?;
    let mut out = DataTransactionScript::new();
    for (step_idx, step) in script.steps().iter().enumerate() {
        let mut builder = out.step(step.name());
        for (op_idx, op) in step.operations().iter().enumerate() {
            let op = if (step_idx, op_idx) == target {
                fact.clone()
            } else {
                op.clone()
            };
            builder = builder.operation(op);
        }
        builder.commit();
    }
    Ok(out)
}

pub(super) fn field_diff(before: &Value, after: &Value) -> Vec<Value> {
    let mut keys = BTreeSet::new();
    if let Some(obj) = before.as_object() {
        keys.extend(obj.keys().cloned());
    }
    if let Some(obj) = after.as_object() {
        keys.extend(obj.keys().cloned());
    }
    keys.into_iter()
        .filter_map(|key| {
            let b = before.get(&key).cloned().unwrap_or(Value::Null);
            let a = after.get(&key).cloned().unwrap_or(Value::Null);
            (b != a).then(|| json!({ "field": key, "before": b, "after": a }))
        })
        .collect()
}

pub(super) fn hashed(surface: &'static str, value: Value) -> Artifact {
    let bytes = serde_json::to_string(&value).unwrap_or_else(|_| "null".into());
    Artifact {
        surface,
        value,
        hash: crate::instrument::window_hash(&bytes),
    }
}

fn command_values<C>(commands: &[ResourceCommand<C>], wire_kind: Option<u64>) -> Vec<Value> {
    commands
        .iter()
        .map(|c| {
            let mut v = json!({
                "op": op_str(c),
                "resource": key_path(c.key()),
            });
            if let Some(kind) = wire_kind {
                v["kind"] = Value::from(kind);
            }
            v
        })
        .collect()
}

fn op_str<C>(c: &ResourceCommand<C>) -> &'static str {
    match c {
        ResourceCommand::Open { .. } => "Open",
        ResourceCommand::Close { .. } => "Close",
        ResourceCommand::Replace { .. } => "Replace",
        ResourceCommand::Refresh { .. } => "Refresh",
    }
}

fn normalize_status_fact(fact: InputFact) -> Result<InputFact> {
    let drive = match fact {
        InputFact::StatusDrive(_) => return Ok(fact),
        InputFact::DistillCompleted {
            session_id,
            window_hash,
            title,
            activity,
            at,
        } => StatusDrive::DistillCompleted {
            session_id,
            title,
            activity,
            window_hash: Some(window_hash),
            at,
        },
        _ => return Err(anyhow::anyhow!("probe: fact is not a status fact")),
    };
    Ok(InputFact::StatusDrive(drive))
}
