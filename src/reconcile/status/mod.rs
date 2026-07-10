//! Per-session kind:30315 status reconciler — the ONE authority that decides
//! WHEN a session's public status is (re)published.
//! One graph owns dedup, refresh, h-tags, and deterministic teardown; the host
//! only signs and enqueues the emitted effects.

mod model;
mod preview;
pub(crate) mod probe;
pub(crate) mod replay;
mod status_build;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use trellis_core::{
    Graph, GraphResult, NodeId, ResourceCommand, ResourceCommandExplanation, Transaction,
    TransactionResult,
};

use crate::domain::Status;
use crate::reconcile::labels::NodeLabels;

use model::{create_session, opts, status_key, SessionNodes, StaticInfo};

/// The graph's in-plan command payload: everything needed to build a kind:30315
/// EXCEPT the expiration, which the host stamps at apply time from its own clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusCommand {
    pub session_id: String,
    pub channels: Vec<String>,
    pub title: String,
    pub activity: String,
    pub busy: bool,
    pub host: String,
    pub slug: String,
    pub pubkey: String,
    pub rel_cwd: String,
    pub dispatch_event: Option<String>,
}

/// Why the reconciler is asking the host to publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishReason {
    /// First status for a session.
    Opened,
    /// The derived content changed (turn edge, distill, channel change).
    Changed,
    /// Content unchanged; only the NIP-40 TTL window was re-armed.
    Refreshed,
}

/// A host effect: the exact status to sign + enqueue on the outbox.
#[derive(Clone, Debug, PartialEq)]
pub enum StatusEffect {
    /// Publish this status (fresh TTL) for the given reason.
    Publish {
        status: Status,
        reason: PublishReason,
    },
    /// Final, immediately-expiring status so peers drop the session at once.
    Expire { status: Status },
}

/// One reconciler tx outcome: effects to apply + the raw receipt (the Slice-8 instrumentation seam).
pub struct StatusOutcome {
    pub effects: Vec<StatusEffect>,
    pub result: TransactionResult<StatusCommand>,
}

#[derive(Clone)]
pub struct StatusReconciler {
    graph: Graph<StatusCommand>,
    ttl_secs: u64,
    refresh_secs: u64,
    sessions: BTreeMap<String, SessionNodes>,
    last: BTreeMap<String, StatusCommand>,
    labels: NodeLabels,
}

impl StatusReconciler {
    /// Build an empty reconciler. `ttl_secs` is the NIP-40 window; `refresh_secs`
    /// is the re-arm cadence (`on_tick` re-arms once per `refresh_secs` bucket).
    pub fn new(ttl_secs: u64, refresh_secs: u64) -> Self {
        Self {
            graph: Graph::<StatusCommand>::new_with_command_type(),
            ttl_secs: ttl_secs.max(1),
            refresh_secs: refresh_secs.max(1),
            sessions: BTreeMap::new(),
            last: BTreeMap::new(),
            labels: NodeLabels::new(),
        }
    }

    pub fn labels(&self) -> &NodeLabels {
        &self.labels
    }

    pub fn graph_node_count(&self) -> usize {
        self.graph.nodes().count()
    }

    /// Daemon constructor: TTL from a `Duration`, cadence from the domain heartbeat.
    pub fn for_ttl(ttl: Duration) -> Self {
        Self::new(ttl.as_secs(), crate::domain::HEARTBEAT_SECS)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn on_session_started(
        &mut self,
        id: &str,
        host: &str,
        slug: &str,
        pubkey: &str,
        rel_cwd: &str,
        channels: BTreeSet<String>,
        working: bool,
        title: &str,
        activity: &str,
        now: u64,
    ) -> GraphResult<StatusOutcome> {
        self.on_session_started_with_dispatch(
            id, host, slug, pubkey, rel_cwd, channels, working, title, activity, None, now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn on_session_started_with_dispatch(
        &mut self,
        id: &str,
        host: &str,
        slug: &str,
        pubkey: &str,
        rel_cwd: &str,
        channels: BTreeSet<String>,
        working: bool,
        title: &str,
        activity: &str,
        dispatch_event: Option<String>,
        now: u64,
    ) -> GraphResult<StatusOutcome> {
        if self.sessions.contains_key(id) {
            return self.empty_commit();
        }
        let info = StaticInfo {
            host: host.to_string(),
            slug: slug.to_string(),
            pubkey: pubkey.to_string(),
            rel_cwd: rel_cwd.to_string(),
            dispatch_event,
        };
        let (nodes, result) = create_session(
            &mut self.graph,
            &mut self.labels,
            id,
            info,
            channels,
            working,
            title,
            activity,
            now / self.refresh_secs,
        )?;
        self.sessions.insert(id.to_string(), nodes);
        let effects = self.translate(&result, now);
        Ok(StatusOutcome { effects, result })
    }

    /// A turn started (busy) / ended (idle: the derive clears the live activity).
    pub fn on_turn_start(&mut self, id: &str, now: u64) -> GraphResult<StatusOutcome> {
        self.mutate(id, now, |tx, n| tx.set_input(n.working, true))
    }

    pub fn on_turn_end(&mut self, id: &str, now: u64) -> GraphResult<StatusOutcome> {
        self.mutate(id, now, |tx, n| tx.set_input(n.working, false))
    }

    /// A distillation completed: the LLM output enters as canonical input.
    pub fn on_distill(
        &mut self,
        id: &str,
        title: &str,
        activity: &str,
        now: u64,
    ) -> GraphResult<StatusOutcome> {
        self.mutate(id, now, |tx, n| {
            tx.set_input(n.title, title.to_string())?;
            tx.set_input(n.activity, activity.to_string())
        })
    }

    pub fn on_channels_changed(
        &mut self,
        id: &str,
        channels: BTreeSet<String>,
        now: u64,
    ) -> GraphResult<StatusOutcome> {
        self.mutate(id, now, move |tx, n| tx.set_input(n.channels, channels))
    }

    /// A clock tick: re-arm the NIP-40 window (a `Refresh`, not a content change)
    /// if `now` crossed a refresh bucket; otherwise nothing.
    pub fn on_tick(&mut self, id: &str, now: u64) -> GraphResult<StatusOutcome> {
        self.mutate(id, now, |_tx, _n| Ok(()))
    }

    /// The session ended (clean exit / pid death): close its scope, which emits a
    /// deterministic closing command translated into a final expiring publish.
    pub fn on_session_ended(&mut self, id: &str, now: u64) -> GraphResult<StatusOutcome> {
        let Some(nodes) = self.sessions.remove(id) else {
            return self.empty_commit();
        };
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        tx.close_scope(nodes.scope)?;
        let result = tx.commit()?;
        drop(tx);
        let effects = self.translate(&result, now);
        Ok(StatusOutcome { effects, result })
    }

    pub fn why_command(&self, id: &str) -> Option<&ResourceCommandExplanation> {
        self.graph.why_resource_command(&status_key(id))
    }

    pub fn activity_input(&self, id: &str) -> Option<NodeId> {
        self.sessions.get(id).map(|n| n.activity_id)
    }

    pub fn assert_oracle(&self) -> GraphResult<()> {
        self.graph.assert_incremental_equals_full()?;
        Ok(())
    }

    /// Stage the caller's input, re-sync the TTL arm bucket, commit, translate.
    fn mutate(
        &mut self,
        id: &str,
        now: u64,
        stage: impl FnOnce(&mut Transaction<'_, StatusCommand>, &SessionNodes) -> GraphResult<()>,
    ) -> GraphResult<StatusOutcome> {
        let Some(nodes) = self.sessions.get(id).copied() else {
            return self.empty_commit();
        };
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        stage(&mut tx, &nodes)?;
        tx.set_input(nodes.arm, now / self.refresh_secs)?;
        let result = tx.commit()?;
        drop(tx);
        let effects = self.translate(&result, now);
        Ok(StatusOutcome { effects, result })
    }

    /// Empty commit for an unknown session, so callers always get a receipt.
    fn empty_commit(&mut self) -> GraphResult<StatusOutcome> {
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        let result = tx.commit()?;
        drop(tx);
        Ok(StatusOutcome {
            effects: Vec::new(),
            result,
        })
    }

    /// Turn the graph's resource plan into host effects, maintaining the
    /// last-published shadow used to build closing/expiring publishes.
    fn translate(
        &mut self,
        result: &TransactionResult<StatusCommand>,
        now: u64,
    ) -> Vec<StatusEffect> {
        let mut effects = Vec::new();
        for command in result.resource_plan.commands() {
            let (cmd, reason) = match command {
                ResourceCommand::Open { command, .. } => (command, PublishReason::Opened),
                ResourceCommand::Replace { command, .. } => (command, PublishReason::Changed),
                ResourceCommand::Refresh { command, .. } => (command, PublishReason::Refreshed),
                ResourceCommand::Close { key, .. } => {
                    if let Some(cmd) = key.segment(1).and_then(|sid| self.last.remove(sid)) {
                        // Expiring publish: activity cleared, expiration = now, but
                        // the last-known `h` tags kept so the retraction lands.
                        effects.push(StatusEffect::Expire {
                            status: self.to_status(&cmd, now, true),
                        });
                    }
                    continue;
                }
            };
            self.last.insert(cmd.session_id.clone(), cmd.clone());
            effects.push(StatusEffect::Publish {
                status: self.to_status(cmd, now, false),
                reason,
            });
        }
        effects
    }

    fn to_status(&self, cmd: &StatusCommand, now: u64, expiring: bool) -> Status {
        status_build::to_status(cmd, self.ttl_secs, now, expiring)
    }
}
