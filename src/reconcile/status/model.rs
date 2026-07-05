//! Data model + pure graph wiring for the status reconciler: the per-session
//! value types, the resource-command payload, the host effects, the map-diff
//! planner, and the one-shot per-session graph construction. No reconciler state
//! lives here — [`super::StatusReconciler`] owns the graph and the shadow map.

use std::collections::{BTreeMap, BTreeSet};

use trellis_core::{
    AuditExplanationLevel, DependencyList, Graph, GraphResult, InputNode, MapDiff, NodeId,
    PlanContext, PlanError, ResourceKey, ResourcePlan, ScopeId, Transaction, TransactionOptions,
    TransactionResult,
};

use crate::reconcile::labels::NodeLabels;

use super::StatusCommand;

/// Static per-session identity/context, fixed for the session's lifetime. Not an
/// input (it never changes), so it never causes a spurious re-publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StaticInfo {
    pub host: String,
    pub slug: String,
    pub pubkey: String,
    pub rel_cwd: String,
}

/// The change-detected half of a session's status. Deliberately EXCLUDES the
/// NIP-40 expiration — TTL re-arm is tracked by `arm` — so an idle heartbeat is
/// never mistaken for a content change. Mirrors the exact wire semantics of the
/// old `status_for`: idle clears the live activity; the title always survives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StatusContent {
    pub channels: Vec<String>,
    pub title: String,
    pub activity: String,
    pub busy: bool,
}

/// One session's full collection value: the content that drives change-detection
/// plus a monotonic re-arm counter. The planner emits a `Replace` when `content`
/// differs and a cheaper `Refresh` when only `arm` advanced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StatusValue {
    pub content: StatusContent,
    pub info: StaticInfo,
    pub arm: u64,
}

/// Per-session graph handles held by the reconciler.
#[derive(Clone, Copy)]
pub(super) struct SessionNodes {
    pub scope: ScopeId,
    pub working: InputNode<bool>,
    pub title: InputNode<String>,
    pub activity: InputNode<String>,
    pub channels: InputNode<BTreeSet<String>>,
    pub arm: InputNode<u64>,
    /// Exposed so instrumentation can attribute a publish to the activity fact.
    pub activity_id: NodeId,
}

/// Resource identity for a session's status: `status/<session_id>`.
pub(super) fn status_key(id: &str) -> ResourceKey {
    ResourceKey::from_segments(["status", id])
}

/// Transaction options with dependency-path audit so a command can be attributed
/// to the exact input fact (e.g. `activity`) that produced it.
pub(super) fn opts() -> TransactionOptions {
    TransactionOptions::default().with_audit_explanations(AuditExplanationLevel::DependencyPaths)
}

/// Build the publish command payload for a session from its collection value.
fn command_of(id: &str, v: &StatusValue) -> StatusCommand {
    StatusCommand {
        session_id: id.to_string(),
        channels: v.content.channels.clone(),
        title: v.content.title.clone(),
        activity: v.content.activity.clone(),
        busy: v.content.busy,
        host: v.info.host.clone(),
        slug: v.info.slug.clone(),
        pubkey: v.info.pubkey.clone(),
        rel_cwd: v.info.rel_cwd.clone(),
    }
}

/// The planner: added → Open, content change → Replace, pure TTL re-arm →
/// Refresh, removed → Close. Only ACTUAL changes reach here (the map diff
/// dedups), so an unchanged commit emits nothing.
fn plan_status(
    ctx: &PlanContext<MapDiff<String, StatusValue>>,
) -> Result<ResourcePlan<StatusCommand>, PlanError> {
    let mut plan = ResourcePlan::new();
    for added in &ctx.diff().added {
        let (id, v) = &added.value;
        plan.open(status_key(id), ctx.scope(), command_of(id, v));
    }
    for updated in &ctx.diff().updated {
        let cmd = command_of(&updated.key, &updated.current);
        if updated.current.content == updated.previous.content {
            // Content identical — only the TTL arm advanced: a cheap re-arm.
            plan.refresh(status_key(&updated.key), ctx.scope(), cmd);
        } else {
            plan.replace(status_key(&updated.key), ctx.scope(), cmd);
        }
    }
    for removed in &ctx.diff().removed {
        plan.close(status_key(&removed.value.0), ctx.scope());
    }
    Ok(plan)
}

/// Create one session's scope, per-field inputs, derived content node, single-
/// entry collection, and planner — then commit the seed so the opening publish
/// is emitted. Returns the handles plus the receipt.
#[allow(clippy::too_many_arguments)]
pub(super) fn create_session(
    graph: &mut Graph<StatusCommand>,
    labels: &mut NodeLabels,
    id: &str,
    info: StaticInfo,
    channels: BTreeSet<String>,
    working: bool,
    title: &str,
    activity: &str,
    arm: u64,
) -> GraphResult<(SessionNodes, TransactionResult<StatusCommand>)> {
    let mut tx = graph.begin_transaction_with_options(opts())?;
    let nodes = stage_session(
        &mut tx, labels, id, info, channels, working, title, activity, arm,
    )?;
    let result = tx.commit()?;
    drop(tx);
    Ok((nodes, result))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn stage_session(
    tx: &mut Transaction<'_, StatusCommand>,
    labels: &mut NodeLabels,
    id: &str,
    info: StaticInfo,
    channels: BTreeSet<String>,
    working: bool,
    title: &str,
    activity: &str,
    arm: u64,
) -> GraphResult<SessionNodes> {
    let scope = tx.create_scope(format!("status-{id}"))?;
    let working_n = tx.input::<bool>(format!("status-{id}-working"))?;
    labels.record(working_n.id(), format!("status/{id}/working"));
    tx.set_input(working_n, working)?;
    let title_n = tx.input::<String>(format!("status-{id}-title"))?;
    labels.record(title_n.id(), format!("status/{id}/title"));
    tx.set_input(title_n, title.to_string())?;
    let activity_n = tx.input::<String>(format!("status-{id}-activity"))?;
    labels.record(activity_n.id(), format!("status/{id}/activity"));
    tx.set_input(activity_n, activity.to_string())?;
    let channels_n = tx.input::<BTreeSet<String>>(format!("status-{id}-channels"))?;
    labels.record(channels_n.id(), format!("status/{id}/channels"));
    tx.set_input(channels_n, channels)?;
    let arm_n = tx.input::<u64>(format!("status-{id}-arm"))?;
    labels.record(arm_n.id(), format!("status/{id}/arm"));
    tx.set_input(arm_n, arm)?;

    let content = tx.derived(
        format!("status-{id}-content"),
        DependencyList::new([
            working_n.id(),
            title_n.id(),
            activity_n.id(),
            channels_n.id(),
        ])?,
        move |ctx| {
            let busy = *ctx.input(working_n)?;
            Ok(StatusContent {
                channels: ctx.input(channels_n)?.iter().cloned().collect(),
                title: ctx.input(title_n)?.clone(),
                activity: if busy {
                    ctx.input(activity_n)?.clone()
                } else {
                    String::new()
                },
                busy,
            })
        },
    )?;

    labels.record(content.id(), format!("status/{id}/content"));

    let key = id.to_string();
    let coll = tx.map_collection::<String, StatusValue>(
        format!("status-{id}-coll"),
        DependencyList::new([content.id(), arm_n.id()])?,
        move |ctx| {
            Ok(BTreeMap::from([(
                key.clone(),
                StatusValue {
                    content: ctx.derived(content)?.clone(),
                    info: info.clone(),
                    arm: *ctx.input(arm_n)?,
                },
            )]))
        },
    )?;
    labels.record(coll.id(), format!("status/{id}/coll"));
    tx.map_resource_planner(coll, scope, plan_status)?;
    Ok(SessionNodes {
        scope,
        working: working_n,
        title: title_n,
        activity: activity_n,
        channels: channels_n,
        arm: arm_n,
        activity_id: activity_n.id(),
    })
}
