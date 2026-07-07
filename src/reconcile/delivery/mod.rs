//! Mention delivery reconciler: the single authority that decides whether a
//! pending inbox row should be pasted into a PTY now, deferred, or cleaned up.

mod model;
pub(crate) mod replay;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use trellis_core::{Graph, GraphResult, ResourceCommand, ResourceCommandCause, TransactionResult};

use crate::reconcile::journal::Timestamp;
use crate::reconcile::labels::{key_path, NodeLabels};

use model::{delivery_key, ensure_session, opts, stage_scan, SessionNodes};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryScanFact {
    pub session_id: String,
    pub pending_event_ids: Vec<String>,
    pub working: bool,
    pub pty_id: Option<String>,
    pub pty_live: bool,
    pub last_injected_at: Option<Timestamp>,
    pub debounce_secs: u64,
    pub force: bool,
    pub at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryAction {
    Inject,
    DeferWorking,
    DeferDebounced,
    DeferNoEndpoint,
    ClearDeadEndpoint,
}

impl DeliveryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inject => "inject",
            Self::DeferWorking => "defer_working",
            Self::DeferDebounced => "defer_debounced",
            Self::DeferNoEndpoint => "defer_no_endpoint",
            Self::ClearDeadEndpoint => "clear_dead_endpoint",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryCommand {
    pub session_id: String,
    pub action: DeliveryAction,
    pub event_ids: Vec<String>,
    pub pty_id: Option<String>,
    pub retry_after_secs: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryEffect {
    Inject {
        session_id: String,
        pty_id: String,
        event_ids: Vec<String>,
    },
    RetryAfter {
        session_id: String,
        delay_secs: u64,
    },
    ClearDeadEndpoint {
        session_id: String,
    },
}

pub struct DeliveryOutcome {
    pub effects: Vec<DeliveryEffect>,
    pub result: TransactionResult<DeliveryCommand>,
}

pub struct DeliveryPreview {
    pub result: TransactionResult<DeliveryCommand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryStateRow {
    pub session: String,
    pub action: String,
    pub event_ids: Vec<String>,
    pub pty_id: Option<String>,
    pub retry_after_secs: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryWhy {
    pub resource_key: String,
    pub last_kind: String,
    pub cause: String,
    pub input_causes: Vec<String>,
}

#[derive(Clone)]
pub struct DeliveryReconciler {
    graph: Graph<DeliveryCommand>,
    sessions: BTreeMap<String, SessionNodes>,
    labels: NodeLabels,
    last: BTreeMap<String, DeliveryCommand>,
    next_seq: u64,
}

impl Default for DeliveryReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl DeliveryReconciler {
    pub fn new() -> Self {
        Self {
            graph: Graph::<DeliveryCommand>::new_with_command_type(),
            sessions: BTreeMap::new(),
            labels: NodeLabels::new(),
            last: BTreeMap::new(),
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

    pub fn scan(&mut self, fact: DeliveryScanFact) -> GraphResult<DeliveryOutcome> {
        let seq = self.next_seq + 1;
        let result = self.stage(&fact, seq, false)?;
        self.next_seq = seq;
        let effects = self.translate(&result);
        Ok(DeliveryOutcome { effects, result })
    }

    pub fn preview_scan(&mut self, fact: &DeliveryScanFact) -> GraphResult<DeliveryPreview> {
        let result = self.stage(fact, self.next_seq + 1, true)?;
        Ok(DeliveryPreview { result })
    }

    pub fn state_rows(&self) -> Vec<DeliveryStateRow> {
        self.last
            .values()
            .map(|cmd| DeliveryStateRow {
                session: cmd.session_id.clone(),
                action: cmd.action.as_str().to_string(),
                event_ids: cmd.event_ids.clone(),
                pty_id: cmd.pty_id.clone(),
                retry_after_secs: cmd.retry_after_secs,
            })
            .collect()
    }

    pub fn explain_delivery(&self, id: &str) -> Option<DeliveryWhy> {
        let why = self.graph.why_resource_command(&delivery_key(id))?;
        Some(DeliveryWhy {
            resource_key: key_path(&delivery_key(id)),
            last_kind: format!("{:?}", why.kind),
            cause: self.cause_label(&why.cause),
            input_causes: self.labels.labels_for(&why.input_causes),
        })
    }

    pub fn assert_oracle(&self) -> GraphResult<()> {
        self.graph.assert_incremental_equals_full()?;
        Ok(())
    }

    fn stage(
        &mut self,
        fact: &DeliveryScanFact,
        seq: u64,
        preview: bool,
    ) -> GraphResult<TransactionResult<DeliveryCommand>> {
        let mut sessions = self.sessions.clone();
        let mut labels = self.labels.clone();
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        let nodes = ensure_session(&mut tx, &mut labels, &mut sessions, &fact.session_id)?;
        stage_scan(&mut tx, &nodes, fact, seq)?;
        let result = if preview { tx.preview()? } else { tx.commit()? };
        if !preview {
            self.sessions = sessions;
            self.labels = labels;
        }
        Ok(result)
    }

    fn translate(&mut self, result: &TransactionResult<DeliveryCommand>) -> Vec<DeliveryEffect> {
        let mut effects = Vec::new();
        for resource in result.resource_plan.commands() {
            let command = match resource {
                ResourceCommand::Open { command, .. }
                | ResourceCommand::Replace { command, .. }
                | ResourceCommand::Refresh { command, .. } => command,
                ResourceCommand::Close { key, .. } => {
                    if let Some(sid) = key.segment(1) {
                        self.last.remove(sid);
                    }
                    continue;
                }
            };
            self.last
                .insert(command.session_id.clone(), command.clone());
            match command.action {
                DeliveryAction::Inject => {
                    if let Some(pty_id) = command.pty_id.clone() {
                        effects.push(DeliveryEffect::Inject {
                            session_id: command.session_id.clone(),
                            pty_id,
                            event_ids: command.event_ids.clone(),
                        });
                    }
                }
                DeliveryAction::DeferDebounced => {
                    if let Some(delay_secs) = command.retry_after_secs {
                        effects.push(DeliveryEffect::RetryAfter {
                            session_id: command.session_id.clone(),
                            delay_secs,
                        });
                    }
                }
                DeliveryAction::ClearDeadEndpoint => {
                    effects.push(DeliveryEffect::ClearDeadEndpoint {
                        session_id: command.session_id.clone(),
                    });
                }
                DeliveryAction::DeferWorking | DeliveryAction::DeferNoEndpoint => {}
            }
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
