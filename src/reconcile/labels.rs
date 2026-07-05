//! Stable, per-surface node-label registry (frontier design §4.2) and the plain
//! flattened commit facts (§4.1) that ride on top of it.
//!
//! Trellis nodes carry a `debug_name`, but core exports no registry, so each
//! reconciler owns its own [`NodeLabels`] map, populated AT NODE CREATION with a
//! stable, semantic path (`status/<session>/activity`,
//! `subscriptions/session/<session>/channels`, `hook/<session>/cursor`). This is
//! the precondition for legible receipts and the all-commit ledger: no consumer
//! ever sees a bare integer node id.
//!
//! [`CommitFacts`] is the Trellis-vocabulary-free flattening of one
//! `TransactionResult` — changed-node arrays resolved THROUGH the label registry,
//! plus the command/output counts and the no-op flag. It carries no Trellis types,
//! so it crosses the host boundary into `instrument`/`state` cleanly.

use std::collections::BTreeMap;

use trellis_core::{
    NodeId, OutputFrame, OutputFrameKind, ResourceCommand, ResourceKey, TransactionResult,
};

/// Render a [`ResourceKey`] as a human path (`status/s1`, `sub/h/general`) by
/// joining its identity segments with `/`. The encoded `as_str()` form escapes
/// multi-segment keys (`segments:status:s1`), so probe output uses THIS instead.
pub fn key_path(key: &ResourceKey) -> String {
    key.segments().collect::<Vec<_>>().join("/")
}

/// A surface's node-id → semantic-label map, built at node creation.
#[derive(Debug, Clone, Default)]
pub struct NodeLabels {
    by_id: BTreeMap<NodeId, String>,
}

impl NodeLabels {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the stable, semantic label for a node the instant it is created.
    /// Later inserts for the same id overwrite (a node is created once, so this is
    /// only exercised by tests that reuse an id deliberately).
    pub fn record(&mut self, id: NodeId, label: impl Into<String>) {
        self.by_id.insert(id, label.into());
    }

    /// The stable label for a node id, or `None` if the id was never registered.
    pub fn label_of(&self, id: NodeId) -> Option<&str> {
        self.by_id.get(&id).map(String::as_str)
    }

    /// Resolve a slice of node ids to labels, in the given order. An unregistered
    /// id degrades to `node:<n>` rather than being dropped, so the array length is
    /// always preserved and a missing label is visible instead of silent.
    pub fn labels_for(&self, ids: &[NodeId]) -> Vec<String> {
        ids.iter()
            .map(|id| {
                self.label_of(*id)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("node:{}", id.get()))
            })
            .collect()
    }

    /// The number of registered labels.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether no labels are registered yet.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Iterate `(id, label)` pairs in stable id order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &str)> {
        self.by_id.iter().map(|(id, label)| (*id, label.as_str()))
    }
}

/// One committed transaction flattened to plain, Trellis-free facts. Changed-node
/// arrays are LABELS (via [`NodeLabels`]); counts and the no-op flag are the
/// value-evidence the all-commit ledger persists for `probe stats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFacts {
    pub transaction_id: i64,
    pub revision: i64,
    /// Changed input nodes, resolved to labels.
    pub changed_inputs: Vec<String>,
    /// Changed derived nodes, resolved to labels.
    pub changed_derived: Vec<String>,
    /// Changed collection nodes, resolved to labels.
    pub changed_collections: Vec<String>,
    /// Number of resource commands this commit emitted.
    pub command_count: i64,
    /// Number of materialized output frames this commit emitted.
    pub output_count: i64,
    /// Payload-free resource command trace.
    pub resource_commands_json: String,
    /// Payload-free output frame trace.
    pub output_frames_json: String,
    /// Total graph node count after the commit.
    pub graph_nodes: i64,
    /// Surface-owned resource count after the commit; callers fill this when
    /// they have a public inventory.
    pub graph_resources: i64,
    /// True when the commit emitted no command and no output frame — it committed
    /// but changed nothing observable (a suppressed publish / no-op recompute).
    pub noop: bool,
}

impl CommitFacts {
    /// Flatten a committed `TransactionResult` through a surface's label registry.
    /// `graph_nodes` is the post-commit node count (the reconciler owns the graph,
    /// so it supplies the count at the boundary).
    pub fn from_result<C>(
        labels: &NodeLabels,
        result: &TransactionResult<C>,
        graph_nodes: usize,
    ) -> Self {
        let command_count = result.resource_plan.commands().len() as i64;
        let output_count = result.output_frames.len() as i64;
        Self {
            transaction_id: result.transaction_id.get() as i64,
            revision: result.revision.get() as i64,
            changed_inputs: labels.labels_for(&result.changed_inputs),
            changed_derived: labels.labels_for(&result.changed_derived_nodes),
            changed_collections: labels.labels_for(&result.changed_collection_nodes),
            resource_commands_json: commands_json(result.resource_plan.commands()),
            output_frames_json: output_frames_json(&result.output_frames),
            command_count,
            output_count,
            graph_nodes: graph_nodes as i64,
            graph_resources: 0,
            noop: command_count == 0 && output_count == 0,
        }
    }
}

fn commands_json<C>(commands: &[ResourceCommand<C>]) -> String {
    #[derive(serde::Serialize)]
    struct Cmd<'a> {
        kind: &'a str,
        key: &'a str,
        reason: &'a str,
    }
    let out: Vec<Cmd> = commands
        .iter()
        .map(|c| {
            let kind = match c {
                ResourceCommand::Open { .. } => "open",
                ResourceCommand::Close { .. } => "close",
                ResourceCommand::Replace { .. } => "replace",
                ResourceCommand::Refresh { .. } => "refresh",
            };
            Cmd {
                kind,
                key: c.key().as_str(),
                reason: kind,
            }
        })
        .collect();
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

fn output_frames_json(frames: &[OutputFrame]) -> String {
    #[derive(serde::Serialize)]
    struct Frame<'a> {
        output_key: u64,
        scope: u64,
        transaction_id: u64,
        revision: u64,
        kind: &'a str,
        reason: Option<&'a str>,
    }
    let out: Vec<Frame> = frames
        .iter()
        .map(|f| {
            let (kind, reason) = match &f.kind {
                OutputFrameKind::Baseline(_) => ("baseline", None),
                OutputFrameKind::Delta(_) => ("delta", None),
                OutputFrameKind::Clear(_) => ("clear", Some("scope_closed")),
                OutputFrameKind::Rebaseline(_, _) => ("rebaseline", Some("requested")),
            };
            Frame {
                output_key: f.output_key.get(),
                scope: f.scope.get(),
                transaction_id: f.transaction_id.get(),
                revision: f.revision.get(),
                kind,
                reason,
            }
        })
        .collect();
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use trellis_core::Graph;

    #[test]
    fn record_then_resolve_round_trips() {
        let mut graph = Graph::<()>::new();
        let mut tx = graph.begin_transaction().unwrap();
        let a = tx.input::<u64>("a").unwrap();
        let b = tx.input::<u64>("b").unwrap();
        tx.commit().unwrap();

        let mut labels = NodeLabels::new();
        labels.record(a.id(), "status/s1/activity");
        labels.record(b.id(), "status/s1/title");

        assert_eq!(labels.label_of(a.id()), Some("status/s1/activity"));
        assert_eq!(labels.len(), 2);
        assert_eq!(
            labels.labels_for(&[a.id(), b.id()]),
            vec![
                "status/s1/activity".to_string(),
                "status/s1/title".to_string()
            ]
        );
    }

    #[test]
    fn unregistered_id_degrades_to_node_marker() {
        let mut graph = Graph::<()>::new();
        let mut tx = graph.begin_transaction().unwrap();
        let a = tx.input::<u64>("a").unwrap();
        tx.commit().unwrap();

        let labels = NodeLabels::new();
        let resolved = labels.labels_for(&[a.id()]);
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].starts_with("node:"));
    }

    #[test]
    fn commit_facts_flags_noop_when_nothing_emitted() {
        // A graph with an input but no planner/output: a commit emits neither a
        // command nor a frame → noop.
        let mut graph = Graph::<()>::new();
        let mut tx = graph.begin_transaction().unwrap();
        let a = tx.input::<BTreeSet<String>>("a").unwrap();
        tx.set_input(a, BTreeSet::new()).unwrap();
        let result = tx.commit().unwrap();
        drop(tx);

        let labels = NodeLabels::new();
        let facts = CommitFacts::from_result(&labels, &result, graph.nodes().count());
        assert!(facts.noop);
        assert_eq!(facts.command_count, 0);
        assert_eq!(facts.output_count, 0);
        assert!(facts.graph_nodes >= 1);
    }
}
