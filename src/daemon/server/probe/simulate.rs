//! `probe simulate`: dry-run an [`InputFact`] via Trellis `tx.preview()`.
//! The daemon-held graph is not mutated; callers get the would-be resource plan
//! and changed labels before any host effect is allowed to run.

use super::{required_str, DaemonState};
use crate::reconcile::journal::{InputFact, StatusDrive};
use crate::reconcile::labels::{key_path, NodeLabels};
use crate::reconcile::status::probe::would_publish;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use trellis_core::{ResourceCommand, TransactionResult};

const STATUS_KIND: u64 = 30315;

pub(super) fn simulate_value(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let Some(fact) = fact_param(params)? else {
        return simulate_legacy_status(state, params);
    };
    let inferred = infer_surface(&fact).context("probe simulate: unsupported InputFact")?;
    let surface = params
        .get("surface")
        .and_then(Value::as_str)
        .unwrap_or(inferred);
    if surface != inferred {
        return Err(anyhow::anyhow!(
            "probe simulate: `{surface}` cannot simulate a `{inferred}` fact"
        ));
    }
    match surface {
        "status" => simulate_status_fact(state, normalize_status_fact(fact)?),
        "turn_lifecycle" => simulate_turn_lifecycle_fact(state, fact),
        "cursor" => simulate_cursor_fact(state, fact),
        "outbox" => simulate_outbox_fact(state, fact),
        "subscriptions" => simulate_subscription_fact(state, fact),
        other => Err(anyhow::anyhow!("probe simulate: unknown surface `{other}`")),
    }
}

fn simulate_status_fact(state: &Arc<DaemonState>, fact: InputFact) -> Result<Value> {
    let mut r = state.status.lock().expect("status mutex poisoned");
    let revision_before = r.revision();
    let preview = r
        .preview_fact(&fact)
        .map_err(|e| anyhow::anyhow!("status preview failed: {e:?}"))?
        .context("probe simulate: fact is not supported by status")?;
    let revision_after = r.revision();
    Ok(plan_value(PlanJson {
        surface: "status",
        fact: serde_json::to_value(&fact).unwrap_or(Value::Null),
        labels: &preview.labels,
        plan: &preview.result,
        revision_before,
        revision_after,
        wire_kind: Some(STATUS_KIND),
        would_publish: Some(would_publish(&preview.result)),
    }))
}

fn simulate_turn_lifecycle_fact(state: &Arc<DaemonState>, fact: InputFact) -> Result<Value> {
    let mut r = state
        .turn_lifecycle
        .lock()
        .expect("turn lifecycle mutex poisoned");
    let revision_before = r.revision();
    let preview = r
        .preview_fact(&fact)
        .map_err(|e| anyhow::anyhow!("turn lifecycle preview failed: {e:?}"))?
        .context("probe simulate: fact is not supported by turn_lifecycle")?;
    let revision_after = r.revision();
    Ok(plan_value(PlanJson {
        surface: "turn_lifecycle",
        fact: serde_json::to_value(&fact).unwrap_or(Value::Null),
        labels: &preview.labels,
        plan: &preview.result,
        revision_before,
        revision_after,
        wire_kind: None,
        would_publish: None,
    }))
}

fn simulate_cursor_fact(state: &Arc<DaemonState>, fact: InputFact) -> Result<Value> {
    let mut r = state.cursor.lock().expect("cursor mutex poisoned");
    let revision_before = r.revision();
    let preview = r
        .preview_fact(&fact)
        .map_err(|e| anyhow::anyhow!("cursor preview failed: {e:?}"))?
        .context("probe simulate: fact is not supported by cursor")?;
    let revision_after = r.revision();
    Ok(plan_value(PlanJson {
        surface: "cursor",
        fact: serde_json::to_value(&fact).unwrap_or(Value::Null),
        labels: &preview.labels,
        plan: &preview.result,
        revision_before,
        revision_after,
        wire_kind: None,
        would_publish: None,
    }))
}

fn simulate_outbox_fact(state: &Arc<DaemonState>, fact: InputFact) -> Result<Value> {
    let mut r = state.outbox.lock().expect("outbox mutex poisoned");
    let revision_before = r.revision();
    let preview = r
        .preview_fact(&fact)
        .map_err(|e| anyhow::anyhow!("outbox preview failed: {e:?}"))?
        .context("probe simulate: fact is not supported by outbox")?;
    let revision_after = r.revision();
    Ok(plan_value(PlanJson {
        surface: "outbox",
        fact: serde_json::to_value(&fact).unwrap_or(Value::Null),
        labels: &preview.labels,
        plan: &preview.result,
        revision_before,
        revision_after,
        wire_kind: None,
        would_publish: None,
    }))
}

fn simulate_subscription_fact(state: &Arc<DaemonState>, fact: InputFact) -> Result<Value> {
    let mut r = state.subs.lock().expect("subscriptions mutex poisoned");
    let revision_before = r.revision();
    let preview = r
        .preview_fact(&fact)
        .map_err(|e| anyhow::anyhow!("subscription preview failed: {e:?}"))?
        .context("probe simulate: fact is not supported by subscriptions")?;
    let revision_after = r.revision();
    Ok(plan_value(PlanJson {
        surface: "subscriptions",
        fact: serde_json::to_value(&fact).unwrap_or(Value::Null),
        labels: &preview.labels,
        plan: &preview.result,
        revision_before,
        revision_after,
        wire_kind: None,
        would_publish: None,
    }))
}

fn simulate_legacy_status(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let surface = params
        .get("surface")
        .and_then(Value::as_str)
        .unwrap_or("status");
    if surface != "status" {
        return Err(anyhow::anyhow!(
            "probe simulate: `{surface}` requires --fact <InputFact-json>"
        ));
    }
    let session = required_str(params, "session")?;
    let title = params.get("title").and_then(Value::as_str);
    let activity = params.get("activity").and_then(Value::as_str);
    let now = params.get("now").and_then(Value::as_u64);

    let mut r = state.status.lock().expect("status mutex poisoned");
    let revision_before = r.revision();
    let plan = r.preview_on_distill(session, title, activity, now)?;
    let revision_after = r.revision();
    let fact = json!({ "kind": "legacy-distill", "session_id": session,
        "title": title, "activity": activity, "now": now });
    Ok(plan_value(PlanJson {
        surface: "status",
        fact,
        labels: r.labels(),
        plan: &plan,
        revision_before,
        revision_after,
        wire_kind: Some(STATUS_KIND),
        would_publish: Some(would_publish(&plan)),
    }))
}

fn fact_param(params: &Value) -> Result<Option<InputFact>> {
    let Some(raw) = params.get("fact") else {
        return Ok(None);
    };
    let fact = match raw {
        Value::String(s) => serde_json::from_str(s).context("probe simulate: invalid fact JSON")?,
        value => serde_json::from_value(value.clone()).context("probe simulate: invalid fact")?,
    };
    Ok(Some(fact))
}

fn infer_surface(fact: &InputFact) -> Option<&'static str> {
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
        _ => return Err(anyhow::anyhow!("probe simulate: fact is not a status fact")),
    };
    Ok(InputFact::StatusDrive(drive))
}

struct PlanJson<'a, C> {
    surface: &'a str,
    fact: Value,
    labels: &'a NodeLabels,
    plan: &'a TransactionResult<C>,
    revision_before: u64,
    revision_after: u64,
    wire_kind: Option<u64>,
    would_publish: Option<bool>,
}

fn plan_value<C>(input: PlanJson<'_, C>) -> Value {
    let commands = command_values(input.plan.resource_plan.commands(), input.wire_kind);
    let would_effect = !commands.is_empty();
    let mut out = json!({
        "verb": "simulate",
        "surface": input.surface,
        "fact": input.fact,
        "commands": commands,
        "changed": input.labels.labels_for(&input.plan.changed_inputs),
        "revision_before": input.revision_before,
        "revision_after": input.revision_after,
        "would_effect": would_effect,
        "ok": true,
    });
    if let Some(would_publish) = input.would_publish {
        out["would_publish"] = Value::Bool(would_publish);
    }
    out
}

fn command_values<C>(commands: &[ResourceCommand<C>], wire_kind: Option<u64>) -> Vec<Value> {
    commands
        .iter()
        .map(|c| {
            let mut v = json!({
                "op": op_str(c),
                "resource": key_path(c.key()),
                "effect": true,
            });
            if let Some(kind) = wire_kind {
                v["kind"] = Value::from(kind);
                v["publish"] = Value::Bool(true);
            }
            v
        })
        .collect()
}

fn op_str<C>(c: &ResourceCommand<C>) -> &'static str {
    match c {
        ResourceCommand::Open { .. } => "Open",
        ResourceCommand::Replace { .. } => "Replace",
        ResourceCommand::Refresh { .. } => "Refresh",
        ResourceCommand::Close { .. } => "Close",
    }
}

#[cfg(test)]
mod tests;
