use super::artifact::Artifact;
use super::DaemonState;
use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::key_path;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use trellis_core::ResourceCommand;

pub(super) fn preview_outbox(state: &Arc<DaemonState>, fact: &InputFact) -> Result<Artifact> {
    let mut r = state.outbox.lock().expect("outbox mutex poisoned");
    let preview = r
        .preview_fact(fact)
        .map_err(|e| anyhow::anyhow!("outbox preview failed: {e:?}"))?
        .context("probe: fact is not supported by outbox")?;
    Ok(super::artifact::hashed(
        "outbox",
        json!({
            "commands": command_values(preview.result.resource_plan.commands()),
            "changed": preview.labels.labels_for(&preview.result.changed_inputs),
            "output_frames": preview.result.output_frames.len(),
        }),
    ))
}

fn command_values(
    commands: &[ResourceCommand<crate::reconcile::outbox::OutboxCommand>],
) -> Vec<Value> {
    commands
        .iter()
        .map(|c| {
            let mut v = json!({
                "op": op_str(c),
                "resource": key_path(c.key()),
            });
            if let Some(cmd) = payload(c) {
                v["local_id"] = Value::from(cmd.local_id);
                v["event_id"] = Value::String(cmd.event_id.clone());
                v["state"] = Value::String(cmd.state.clone());
                v["retries"] = Value::from(cmd.retries);
                v["last_error"] = cmd
                    .last_error
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                v["action"] = Value::String(cmd.action.as_str().to_string());
                v["source_ref"] = Value::String(cmd.source_ref.clone());
            }
            v
        })
        .collect()
}

fn payload(
    command: &ResourceCommand<crate::reconcile::outbox::OutboxCommand>,
) -> Option<&crate::reconcile::outbox::OutboxCommand> {
    match command {
        ResourceCommand::Open { command, .. }
        | ResourceCommand::Replace { command, .. }
        | ResourceCommand::Refresh { command, .. } => Some(command),
        ResourceCommand::Close { .. } => None,
    }
}

fn op_str<C>(c: &ResourceCommand<C>) -> &'static str {
    match c {
        ResourceCommand::Open { .. } => "Open",
        ResourceCommand::Close { .. } => "Close",
        ResourceCommand::Replace { .. } => "Replace",
        ResourceCommand::Refresh { .. } => "Refresh",
    }
}
