use std::collections::BTreeMap;

use trellis_core::{
    AuditExplanationLevel, DependencyList, GraphResult, InputNode, MapDiff, PlanContext, PlanError,
    ResourceKey, ResourcePlan, Transaction, TransactionOptions,
};

use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::NodeLabels;

use super::{OutboxAction, OutboxCommand, OutboxSeed};

const RESULT_NONE: i64 = -1;
const RESULT_FAILED: i64 = 0;
const RESULT_ACCEPTED: i64 = 1;

#[derive(Clone, Copy)]
pub(crate) struct EntryNodes {
    pub(super) event_id: InputNode<String>,
    pub(super) event_hash: InputNode<String>,
    pub(super) source_surface: InputNode<String>,
    pub(super) source_ref: InputNode<String>,
    pub(super) retries: InputNode<i64>,
    pub(super) result: InputNode<i64>,
    pub(super) error: InputNode<String>,
    pub(super) seq: InputNode<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Decision {
    local_id: i64,
    event_id: String,
    event_hash: String,
    source_surface: String,
    source_ref: String,
    state: String,
    retries: i64,
    last_error: Option<String>,
    action: OutboxAction,
    seq: u64,
}

pub(crate) fn fact_seed(fact: &InputFact) -> Option<OutboxSeed> {
    match fact {
        InputFact::OutboxEnqueueApplied {
            local_id,
            event_id,
            event_hash,
            source_surface,
            source_ref,
            ..
        } => Some(OutboxSeed {
            local_id: *local_id,
            event_id: event_id.clone(),
            event_hash: event_hash.clone(),
            source_surface: source_surface.clone(),
            source_ref: source_ref.clone(),
            retries: 0,
        }),
        InputFact::RelayPublishAccepted {
            local_id, event_id, ..
        } => Some(OutboxSeed {
            local_id: *local_id,
            event_id: event_id.clone(),
            event_hash: String::new(),
            source_surface: String::new(),
            source_ref: String::new(),
            retries: 0,
        }),
        _ => None,
    }
}

pub(crate) fn outbox_key(local_id: i64) -> ResourceKey {
    ResourceKey::from_segments(["outbox", &local_id.to_string()])
}

pub(crate) fn opts() -> TransactionOptions {
    TransactionOptions::default().with_audit_explanations(AuditExplanationLevel::DependencyPaths)
}

pub(crate) fn ensure_entry(
    tx: &mut Transaction<'_, OutboxCommand>,
    labels: &mut NodeLabels,
    nodes_by_id: &mut BTreeMap<i64, EntryNodes>,
    seed: &OutboxSeed,
) -> GraphResult<EntryNodes> {
    if let Some(nodes) = nodes_by_id.get(&seed.local_id).copied() {
        return Ok(nodes);
    }
    let nodes = stage_entry(tx, labels, seed)?;
    nodes_by_id.insert(seed.local_id, nodes);
    Ok(nodes)
}

pub(crate) fn stage_fact(
    tx: &mut Transaction<'_, OutboxCommand>,
    nodes: &EntryNodes,
    seed: &OutboxSeed,
    fact: &InputFact,
    seq: u64,
) -> GraphResult<()> {
    match fact {
        InputFact::OutboxEnqueueApplied { .. } => {
            tx.set_input(nodes.event_id, seed.event_id.clone())?;
            tx.set_input(nodes.event_hash, seed.event_hash.clone())?;
            tx.set_input(nodes.source_surface, seed.source_surface.clone())?;
            tx.set_input(nodes.source_ref, seed.source_ref.clone())?;
            tx.set_input(nodes.retries, seed.retries)?;
            tx.set_input(nodes.result, RESULT_NONE)?;
            tx.set_input(nodes.error, String::new())?;
            tx.set_input(nodes.seq, seq)?;
        }
        InputFact::RelayPublishAccepted {
            accepted, error, ..
        } => {
            tx.set_input(nodes.event_id, seed.event_id.clone())?;
            tx.set_input(nodes.retries, seed.retries)?;
            tx.set_input(
                nodes.result,
                if *accepted {
                    RESULT_ACCEPTED
                } else {
                    RESULT_FAILED
                },
            )?;
            tx.set_input(nodes.error, error.clone().unwrap_or_default())?;
            tx.set_input(nodes.seq, seq)?;
        }
        _ => {}
    }
    Ok(())
}

fn stage_entry(
    tx: &mut Transaction<'_, OutboxCommand>,
    labels: &mut NodeLabels,
    seed: &OutboxSeed,
) -> GraphResult<EntryNodes> {
    let id = seed.local_id;
    let scope = tx.create_scope(format!("outbox-{id}"))?;
    let event_id = input(tx, labels, id, "event_id", seed.event_id.clone())?;
    let event_hash = input(tx, labels, id, "event_hash", seed.event_hash.clone())?;
    let source_surface = input(
        tx,
        labels,
        id,
        "source_surface",
        seed.source_surface.clone(),
    )?;
    let source_ref = input(tx, labels, id, "source_ref", seed.source_ref.clone())?;
    let retries = input(tx, labels, id, "retries", seed.retries)?;
    let result = input(tx, labels, id, "result", RESULT_NONE)?;
    let error = input(tx, labels, id, "error", String::new())?;
    let seq = input(tx, labels, id, "request_seq", 0u64)?;
    let nodes = EntryNodes {
        event_id,
        event_hash,
        source_surface,
        source_ref,
        retries,
        result,
        error,
        seq,
    };
    let decision = decision_node(tx, labels, id, nodes)?;
    let coll = tx.map_collection::<i64, Decision>(
        format!("outbox-{id}-coll"),
        DependencyList::new([decision.id()])?,
        move |ctx| Ok(BTreeMap::from([(id, ctx.derived(decision)?.clone())])),
    )?;
    labels.record(coll.id(), format!("outbox/{id}/coll"));
    tx.map_resource_planner(coll, scope, plan_outbox)?;
    Ok(nodes)
}

fn input<T: Clone + PartialEq + Send + Sync + 'static>(
    tx: &mut Transaction<'_, OutboxCommand>,
    labels: &mut NodeLabels,
    id: i64,
    name: &str,
    value: T,
) -> GraphResult<InputNode<T>> {
    let node = tx.input::<T>(format!("outbox-{id}-{name}"))?;
    labels.record(node.id(), format!("outbox/{id}/{name}"));
    tx.set_input(node, value)?;
    Ok(node)
}

fn decision_node(
    tx: &mut Transaction<'_, OutboxCommand>,
    labels: &mut NodeLabels,
    id: i64,
    nodes: EntryNodes,
) -> GraphResult<trellis_core::DerivedNode<Decision>> {
    let decision = tx.derived(
        format!("outbox-{id}-decision"),
        DependencyList::new([
            nodes.event_id.id(),
            nodes.event_hash.id(),
            nodes.source_surface.id(),
            nodes.source_ref.id(),
            nodes.retries.id(),
            nodes.result.id(),
            nodes.error.id(),
            nodes.seq.id(),
        ])?,
        move |ctx| {
            let result = *ctx.input(nodes.result)?;
            let retries = *ctx.input(nodes.retries)?;
            let action = match result {
                RESULT_ACCEPTED => OutboxAction::MarkPublished,
                RESULT_FAILED => OutboxAction::MarkFailed,
                _ => OutboxAction::TrackPending,
            };
            Ok(Decision {
                local_id: id,
                event_id: ctx.input(nodes.event_id)?.clone(),
                event_hash: ctx.input(nodes.event_hash)?.clone(),
                source_surface: ctx.input(nodes.source_surface)?.clone(),
                source_ref: ctx.input(nodes.source_ref)?.clone(),
                state: if result == RESULT_ACCEPTED {
                    "published".into()
                } else {
                    "pending".into()
                },
                retries: if result == RESULT_FAILED {
                    retries.saturating_add(1)
                } else {
                    retries
                },
                last_error: if result == RESULT_FAILED {
                    Some(ctx.input(nodes.error)?.clone())
                } else {
                    None
                },
                action,
                seq: *ctx.input(nodes.seq)?,
            })
        },
    )?;
    labels.record(decision.id(), format!("outbox/{id}/decision"));
    Ok(decision)
}

fn plan_outbox(
    ctx: &PlanContext<MapDiff<i64, Decision>>,
) -> Result<ResourcePlan<OutboxCommand>, PlanError> {
    let mut plan = ResourcePlan::new();
    for added in &ctx.diff().added {
        let (id, decision) = &added.value;
        plan.open(outbox_key(*id), ctx.scope(), command_of(decision));
    }
    for updated in &ctx.diff().updated {
        plan.replace(
            outbox_key(updated.key),
            ctx.scope(),
            command_of(&updated.current),
        );
    }
    Ok(plan)
}

fn command_of(decision: &Decision) -> OutboxCommand {
    let _ = decision.seq;
    OutboxCommand {
        local_id: decision.local_id,
        event_id: decision.event_id.clone(),
        event_hash: decision.event_hash.clone(),
        source_surface: decision.source_surface.clone(),
        source_ref: decision.source_ref.clone(),
        state: decision.state.clone(),
        retries: decision.retries,
        last_error: decision.last_error.clone(),
        action: decision.action.clone(),
    }
}
