//! Host seam for the authoritative mention-delivery graph.
//!
//! The reconciler decides inject/defer/cleanup from explicit scan facts. This
//! seam records the commit, receipt, and replay capsule before the PTY host
//! applies the returned effects.

use std::sync::Mutex;

use anyhow::Result;
use serde::Serialize;
use trellis_core::{ResourceCommand, TransactionResult};

use crate::reconcile::{
    CommitFacts, DeliveryCommand, DeliveryEffect, DeliveryReconciler, DeliveryScanFact, InputFact,
};
use crate::state::receipts::NewReceipt;
use crate::state::Store;

pub(crate) fn drive(
    delivery: &Mutex<DeliveryReconciler>,
    store: &Mutex<Store>,
    trigger: &str,
    fact: DeliveryScanFact,
) -> Result<Vec<DeliveryEffect>> {
    let start = std::time::Instant::now();
    let trigger_ref = fact.pubkey.clone();
    let (preview, outcome, commit) = {
        let mut r = delivery.lock().expect("delivery mutex poisoned");
        let preview = r
            .preview_scan(&fact)
            .map_err(|e| anyhow::anyhow!("delivery preview failed: {e:?}"))?
            .result;
        let outcome = r
            .scan(fact.clone())
            .map_err(|e| anyhow::anyhow!("delivery drive failed: {e:?}"))?;
        let mut commit =
            CommitFacts::from_result(r.labels(), &outcome.result, r.graph_node_count());
        commit.graph_resources = r.state_rows().len() as i64;
        (preview, outcome, commit)
    };
    if !crate::reconcile::preview::command_plans_match(
        preview.resource_plan.commands(),
        outcome.result.resource_plan.commands(),
    ) {
        anyhow::bail!("delivery effects blocked: committed plan was not previewed first");
    }
    let created_at = crate::instrument::now_millis();
    let duration_us = start.elapsed().as_micros() as i64;
    {
        let g = store.lock().expect("store mutex poisoned");
        crate::instrument::record_commit(
            &g,
            "delivery",
            trigger,
            Some(trigger_ref.as_str()),
            &commit,
            duration_us,
            created_at,
        );
        record_delivery_receipts(&g, &outcome.result, created_at);
        crate::replay_capsules::record(
            &g,
            "delivery",
            trigger,
            Some(trigger_ref.as_str()),
            InputFact::DeliveryScan(fact),
            created_at,
        );
    }
    Ok(outcome.effects)
}

fn record_delivery_receipts(
    store: &Store,
    result: &TransactionResult<DeliveryCommand>,
    created_at: i64,
) {
    for command in planned_commands(result) {
        for event_id in &command.event_ids {
            let row = NewReceipt {
                surface: "delivery".into(),
                transaction_id: result.transaction_id.get() as i64,
                revision: result.revision.get() as i64,
                changed_summary: changed_summary(result, command),
                commands: commands_json(result.resource_plan.commands()),
                artifact_ref: Some(event_id.clone()),
                created_at,
            };
            crate::instrument::record_receipt(store, row);
        }
    }
}

fn planned_commands(result: &TransactionResult<DeliveryCommand>) -> Vec<&DeliveryCommand> {
    result
        .resource_plan
        .commands()
        .iter()
        .filter_map(|command| match command {
            ResourceCommand::Open { command, .. }
            | ResourceCommand::Replace { command, .. }
            | ResourceCommand::Refresh { command, .. } => Some(command),
            ResourceCommand::Close { .. } => None,
        })
        .collect()
}

fn changed_summary(
    result: &TransactionResult<DeliveryCommand>,
    command: &DeliveryCommand,
) -> String {
    #[derive(Serialize)]
    struct Summary<'a> {
        inputs: Vec<u64>,
        derived: Vec<u64>,
        collections: Vec<u64>,
        pubkey: &'a str,
        event_ids: &'a [String],
        action: &'a str,
        retry_after_secs: Option<u64>,
    }
    let ids = |nodes: &[trellis_core::NodeId]| nodes.iter().map(|n| n.get()).collect();
    serde_json::to_string(&Summary {
        inputs: ids(&result.changed_inputs),
        derived: ids(&result.changed_derived_nodes),
        collections: ids(&result.changed_collection_nodes),
        pubkey: &command.pubkey,
        event_ids: &command.event_ids,
        action: command.action.as_str(),
        retry_after_secs: command.retry_after_secs,
    })
    .unwrap_or_else(|_| "{}".into())
}

fn commands_json(commands: &[ResourceCommand<DeliveryCommand>]) -> String {
    #[derive(Serialize)]
    struct Cmd<'a> {
        kind: &'a str,
        key: &'a str,
        reason: &'a str,
        event_ids: &'a [String],
        retry_after_secs: Option<u64>,
    }
    let out: Vec<Cmd> = commands
        .iter()
        .filter_map(|c| {
            let (kind, command) = match c {
                ResourceCommand::Open { command, .. } => ("open", command),
                ResourceCommand::Replace { command, .. } => ("replace", command),
                ResourceCommand::Refresh { command, .. } => ("refresh", command),
                ResourceCommand::Close { .. } => return None,
            };
            Some(Cmd {
                kind,
                key: c.key().as_str(),
                reason: command.action.as_str(),
                event_ids: &command.event_ids,
                retry_after_secs: command.retry_after_secs,
            })
        })
        .collect();
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}
