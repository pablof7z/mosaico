use std::collections::BTreeMap;

use trellis_core::{
    AuditExplanationLevel, DependencyList, GraphResult, InputNode, MapDiff, PlanContext, PlanError,
    ResourceKey, ResourcePlan, Transaction, TransactionOptions,
};

use crate::reconcile::labels::NodeLabels;

use super::{DeliveryAction, DeliveryCommand, DeliveryScanFact};

#[derive(Clone, Copy)]
pub(crate) struct SessionNodes {
    pending: InputNode<Vec<String>>,
    working: InputNode<bool>,
    pty_id: InputNode<Option<String>>,
    pty_live: InputNode<bool>,
    last_injected_at: InputNode<u64>,
    debounce_secs: InputNode<u64>,
    now: InputNode<u64>,
    force: InputNode<bool>,
    seq: InputNode<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Decision {
    command: Option<DeliveryCommand>,
    seq: u64,
}

pub(crate) fn delivery_key(id: &str) -> ResourceKey {
    ResourceKey::from_segments(["delivery", id])
}

pub(crate) fn opts() -> TransactionOptions {
    TransactionOptions::default().with_audit_explanations(AuditExplanationLevel::DependencyPaths)
}

pub(crate) fn ensure_session(
    tx: &mut Transaction<'_, DeliveryCommand>,
    labels: &mut NodeLabels,
    sessions: &mut BTreeMap<String, SessionNodes>,
    session_id: &str,
) -> GraphResult<SessionNodes> {
    if let Some(nodes) = sessions.get(session_id).copied() {
        return Ok(nodes);
    }
    let nodes = stage_session(tx, labels, session_id)?;
    sessions.insert(session_id.to_string(), nodes);
    Ok(nodes)
}

pub(crate) fn stage_scan(
    tx: &mut Transaction<'_, DeliveryCommand>,
    nodes: &SessionNodes,
    fact: &DeliveryScanFact,
    seq: u64,
) -> GraphResult<()> {
    tx.set_input(nodes.pending, fact.pending_event_ids.clone())?;
    tx.set_input(nodes.working, fact.working)?;
    tx.set_input(nodes.pty_id, fact.pty_id.clone())?;
    tx.set_input(nodes.pty_live, fact.pty_live)?;
    tx.set_input(nodes.last_injected_at, fact.last_injected_at.unwrap_or(0))?;
    tx.set_input(nodes.debounce_secs, fact.debounce_secs)?;
    tx.set_input(nodes.now, fact.at)?;
    tx.set_input(nodes.force, fact.force)?;
    tx.set_input(nodes.seq, seq)
}

fn stage_session(
    tx: &mut Transaction<'_, DeliveryCommand>,
    labels: &mut NodeLabels,
    session_id: &str,
) -> GraphResult<SessionNodes> {
    let scope = tx.create_scope(format!("delivery-{session_id}"))?;
    let pending = input(
        tx,
        labels,
        session_id,
        "pending_event_ids",
        Vec::<String>::new(),
    )?;
    let working = input(tx, labels, session_id, "working", false)?;
    let pty_id = input(tx, labels, session_id, "pty_id", None::<String>)?;
    let pty_live = input(tx, labels, session_id, "pty_live", false)?;
    let last = input(tx, labels, session_id, "last_injected_at", 0u64)?;
    let debounce = input(tx, labels, session_id, "debounce_secs", 0u64)?;
    let now = input(tx, labels, session_id, "now", 0u64)?;
    let force = input(tx, labels, session_id, "force", false)?;
    let seq = input(tx, labels, session_id, "request_seq", 0u64)?;
    let nodes = SessionNodes {
        pending,
        working,
        pty_id,
        pty_live,
        last_injected_at: last,
        debounce_secs: debounce,
        now,
        force,
        seq,
    };
    let decision = decision_node(tx, labels, session_id, nodes)?;
    let id = session_id.to_string();
    let coll = tx.map_collection::<String, Decision>(
        format!("delivery-{session_id}-coll"),
        DependencyList::new([decision.id()])?,
        move |ctx| {
            let current = ctx.derived(decision)?.clone();
            if current.command.is_some() {
                Ok(BTreeMap::from([(id.clone(), current)]))
            } else {
                Ok(BTreeMap::new())
            }
        },
    )?;
    labels.record(coll.id(), format!("delivery/{session_id}/coll"));
    tx.map_resource_planner(coll, scope, plan_delivery)?;
    Ok(nodes)
}

fn input<T: Clone + PartialEq + Send + Sync + 'static>(
    tx: &mut Transaction<'_, DeliveryCommand>,
    labels: &mut NodeLabels,
    session_id: &str,
    name: &str,
    value: T,
) -> GraphResult<InputNode<T>> {
    let node = tx.input::<T>(format!("delivery-{session_id}-{name}"))?;
    labels.record(node.id(), format!("delivery/{session_id}/{name}"));
    tx.set_input(node, value)?;
    Ok(node)
}

fn decision_node(
    tx: &mut Transaction<'_, DeliveryCommand>,
    labels: &mut NodeLabels,
    id: &str,
    nodes: SessionNodes,
) -> GraphResult<trellis_core::DerivedNode<Decision>> {
    let session_id = id.to_string();
    let decision = tx.derived(
        format!("delivery-{id}-decision"),
        DependencyList::new([
            nodes.pending.id(),
            nodes.working.id(),
            nodes.pty_id.id(),
            nodes.pty_live.id(),
            nodes.last_injected_at.id(),
            nodes.debounce_secs.id(),
            nodes.now.id(),
            nodes.force.id(),
            nodes.seq.id(),
        ])?,
        move |ctx| {
            let command = decide(
                &session_id,
                ctx.input(nodes.pending)?.clone(),
                *ctx.input(nodes.working)?,
                ctx.input(nodes.pty_id)?.clone(),
                *ctx.input(nodes.pty_live)?,
                *ctx.input(nodes.last_injected_at)?,
                *ctx.input(nodes.debounce_secs)?,
                *ctx.input(nodes.now)?,
                *ctx.input(nodes.force)?,
            );
            Ok(Decision {
                command,
                seq: *ctx.input(nodes.seq)?,
            })
        },
    )?;
    labels.record(decision.id(), format!("delivery/{id}/decision"));
    Ok(decision)
}

#[allow(clippy::too_many_arguments)]
fn decide(
    session_id: &str,
    event_ids: Vec<String>,
    working: bool,
    pty_id: Option<String>,
    pty_live: bool,
    last_injected_at: u64,
    debounce_secs: u64,
    now: u64,
    force: bool,
) -> Option<DeliveryCommand> {
    if event_ids.is_empty() {
        return None;
    }
    if working && !force {
        return Some(command(
            session_id,
            DeliveryAction::DeferWorking,
            event_ids,
            None,
            None,
        ));
    }
    let Some(pty_id) = pty_id else {
        return Some(command(
            session_id,
            DeliveryAction::DeferNoEndpoint,
            event_ids,
            None,
            None,
        ));
    };
    if !pty_live {
        return Some(command(
            session_id,
            DeliveryAction::ClearDeadEndpoint,
            event_ids,
            Some(pty_id),
            None,
        ));
    }
    let elapsed = now.saturating_sub(last_injected_at);
    if !force && last_injected_at > 0 && elapsed < debounce_secs {
        let retry_after = debounce_secs.saturating_sub(elapsed).max(1);
        return Some(command(
            session_id,
            DeliveryAction::DeferDebounced,
            event_ids,
            Some(pty_id),
            Some(retry_after),
        ));
    }
    Some(command(
        session_id,
        DeliveryAction::Inject,
        event_ids,
        Some(pty_id),
        None,
    ))
}

fn command(
    session_id: &str,
    action: DeliveryAction,
    event_ids: Vec<String>,
    pty_id: Option<String>,
    retry_after_secs: Option<u64>,
) -> DeliveryCommand {
    DeliveryCommand {
        session_id: session_id.to_string(),
        action,
        event_ids,
        pty_id,
        retry_after_secs,
    }
}

fn plan_delivery(
    ctx: &PlanContext<MapDiff<String, Decision>>,
) -> Result<ResourcePlan<DeliveryCommand>, PlanError> {
    let mut plan = ResourcePlan::new();
    for added in &ctx.diff().added {
        let (id, decision) = &added.value;
        if let Some(command) = command_of(decision) {
            plan.open(delivery_key(id), ctx.scope(), command);
        }
    }
    for updated in &ctx.diff().updated {
        if let Some(command) = command_of(&updated.current) {
            plan.replace(delivery_key(&updated.key), ctx.scope(), command);
        }
    }
    for removed in &ctx.diff().removed {
        plan.close(delivery_key(&removed.value.0), ctx.scope());
    }
    Ok(plan)
}

fn command_of(decision: &Decision) -> Option<DeliveryCommand> {
    let _ = decision.seq;
    decision.command.clone()
}
