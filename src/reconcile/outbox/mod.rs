//! Outbox reconciler: tracks enqueue/result facts so relay publish outcomes feed
//! back into Trellis before the host mutates the durable queue row.

mod model;
pub(crate) mod replay;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use trellis_core::{Graph, GraphResult, ResourceCommand, ResourceCommandCause, TransactionResult};

use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::{key_path, NodeLabels};

use model::{ensure_entry, fact_seed, opts, outbox_key, stage_fact, EntryNodes};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxSeed {
    pub local_id: i64,
    pub event_id: String,
    pub event_hash: String,
    pub source_surface: String,
    pub source_ref: String,
    pub retries: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboxAction {
    TrackPending,
    MarkPublished,
    MarkFailed,
}

impl OutboxAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TrackPending => "TrackPending",
            Self::MarkPublished => "MarkPublished",
            Self::MarkFailed => "MarkFailed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxCommand {
    pub local_id: i64,
    pub event_id: String,
    pub event_hash: String,
    pub source_surface: String,
    pub source_ref: String,
    pub state: String,
    pub retries: i64,
    pub last_error: Option<String>,
    pub action: OutboxAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboxEffect {
    None,
    MarkPublished { local_id: i64 },
    MarkFailed { local_id: i64, error: String },
}

pub struct OutboxOutcome {
    pub effects: Vec<OutboxEffect>,
    pub result: TransactionResult<OutboxCommand>,
}

pub struct OutboxPreview {
    pub labels: NodeLabels,
    pub result: TransactionResult<OutboxCommand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxStateRow {
    pub local_id: i64,
    pub event_id: String,
    pub state: String,
    pub retries: i64,
    pub last_error: Option<String>,
    pub source_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxWhy {
    pub resource_key: String,
    pub last_kind: String,
    pub cause: String,
    pub input_causes: Vec<String>,
}

#[derive(Clone)]
pub struct OutboxReconciler {
    graph: Graph<OutboxCommand>,
    nodes: BTreeMap<i64, EntryNodes>,
    entries: BTreeMap<i64, OutboxCommand>,
    labels: NodeLabels,
    next_seq: u64,
}

impl Default for OutboxReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl OutboxReconciler {
    pub fn new() -> Self {
        Self {
            graph: Graph::<OutboxCommand>::new_with_command_type(),
            nodes: BTreeMap::new(),
            entries: BTreeMap::new(),
            labels: NodeLabels::new(),
            next_seq: 0,
        }
    }

    pub fn labels(&self) -> &NodeLabels {
        &self.labels
    }

    pub fn revision(&self) -> u64 {
        self.graph.revision().get()
    }

    pub fn graph_node_count(&self) -> usize {
        self.graph.nodes().count()
    }

    pub fn drive(&mut self, fact: InputFact) -> GraphResult<OutboxOutcome> {
        let seq = self.next_seq + 1;
        let seed = self.seed_for(&fact);
        let (result, _) = self.stage(seed, &fact, seq, false)?;
        self.next_seq = seq;
        let effects = self.translate(&result);
        Ok(OutboxOutcome { effects, result })
    }

    pub fn preview_fact(&mut self, fact: &InputFact) -> GraphResult<Option<OutboxPreview>> {
        if fact_seed(fact).is_none() {
            return Ok(None);
        }
        let seed = self.seed_for(fact);
        let (result, labels) = self.stage(seed, fact, self.next_seq + 1, true)?;
        Ok(Some(OutboxPreview { labels, result }))
    }

    pub fn state_rows(&self) -> Vec<OutboxStateRow> {
        self.entries
            .values()
            .map(|cmd| OutboxStateRow {
                local_id: cmd.local_id,
                event_id: cmd.event_id.clone(),
                state: cmd.state.clone(),
                retries: cmd.retries,
                last_error: cmd.last_error.clone(),
                source_ref: cmd.source_ref.clone(),
            })
            .collect()
    }

    pub fn explain_outbox(&self, local_id: i64) -> Option<OutboxWhy> {
        let why = self.graph.why_resource_command(&outbox_key(local_id))?;
        Some(OutboxWhy {
            resource_key: key_path(&outbox_key(local_id)),
            last_kind: format!("{:?}", why.kind),
            cause: self.cause_label(&why.cause),
            input_causes: self.labels.labels_for(&why.input_causes),
        })
    }

    pub fn assert_oracle(&self) -> GraphResult<()> {
        self.graph.assert_incremental_equals_full()?;
        Ok(())
    }

    fn seed_for(&self, fact: &InputFact) -> OutboxSeed {
        let Some(mut seed) = fact_seed(fact) else {
            unreachable!("outbox facts are classified before seeding")
        };
        if let Some(current) = self.entries.get(&seed.local_id) {
            seed.event_hash = current.event_hash.clone();
            seed.source_surface = current.source_surface.clone();
            seed.source_ref = current.source_ref.clone();
            seed.retries = current.retries;
        }
        seed
    }

    fn stage(
        &mut self,
        seed: OutboxSeed,
        fact: &InputFact,
        seq: u64,
        preview: bool,
    ) -> GraphResult<(TransactionResult<OutboxCommand>, NodeLabels)> {
        let mut nodes_by_id = self.nodes.clone();
        let mut labels = self.labels.clone();
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        let nodes = ensure_entry(&mut tx, &mut labels, &mut nodes_by_id, &seed)?;
        stage_fact(&mut tx, &nodes, &seed, fact, seq)?;
        let result = if preview { tx.preview()? } else { tx.commit()? };
        if !preview {
            self.nodes = nodes_by_id;
            self.labels = labels.clone();
        }
        Ok((result, labels))
    }

    fn translate(&mut self, result: &TransactionResult<OutboxCommand>) -> Vec<OutboxEffect> {
        let mut effects = Vec::new();
        for command in result.resource_plan.commands() {
            let cmd = match command {
                ResourceCommand::Open { command, .. }
                | ResourceCommand::Replace { command, .. }
                | ResourceCommand::Refresh { command, .. } => command,
                ResourceCommand::Close { .. } => continue,
            };
            self.entries.insert(cmd.local_id, cmd.clone());
            effects.push(match cmd.action {
                OutboxAction::TrackPending => OutboxEffect::None,
                OutboxAction::MarkPublished => OutboxEffect::MarkPublished {
                    local_id: cmd.local_id,
                },
                OutboxAction::MarkFailed => OutboxEffect::MarkFailed {
                    local_id: cmd.local_id,
                    error: cmd
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "publish failed".into()),
                },
            });
        }
        effects
    }

    fn cause_label(&self, cause: &ResourceCommandCause) -> String {
        match cause {
            ResourceCommandCause::Planner { collection } => format!(
                "planner: {}",
                self.labels
                    .label_of(*collection)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("node:{}", collection.get()))
            ),
            ResourceCommandCause::ScopeClosed { scope } => {
                format!("scope-closed: {}", scope.get())
            }
        }
    }
}
