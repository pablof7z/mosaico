//! Advisory session-start reconciler.
//!
//! This surface is capped at advisory: Trellis derives staged intents that
//! `rpc_session_start` consults, but the daemon still performs every DB, relay,
//! tmux, signer, and spawn effect imperatively.

mod model;
pub(crate) mod replay;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use trellis_core::{Graph, GraphResult, ResourceCommand, ResourceCommandCause, TransactionResult};

use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::{key_path, NodeLabels};
use crate::reconcile::SessionStartRequestFact;

use model::{ensure_session, fact_session_id, opts, session_key, stage_fact, SessionNodes};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRowIntent {
    pub harness: String,
    pub external_id_kind: String,
    pub external_id: String,
    pub agent_pubkey: String,
    pub agent_slug: String,
    pub channel_h: String,
    pub child_pid: Option<i32>,
    pub resume_id: String,
    pub now: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelReadyIntent {
    pub channel_h: String,
    pub work_root: String,
    pub room_parent: Option<String>,
    pub base_pubkey: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineStartIntent {
    pub session_id: String,
    pub channel_h: String,
    pub rel_cwd: String,
    pub watch_pid: Option<i32>,
    pub signer_label: String,
    pub signer_pubkey: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStartPlan {
    pub row: SessionRowIntent,
    pub channel_ready: Option<ChannelReadyIntent>,
    pub admit_pubkey: Option<String>,
    pub tmux_pane: Option<String>,
    pub ring_doorbell: bool,
    pub notify_outbox: bool,
    pub ensure_subscription: bool,
    pub replay_chat: bool,
    pub spawn: Option<EngineStartIntent>,
    pub emit_tail: bool,
    pub reassert: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionStartAction {
    Execute,
    Reassert,
    RecordStarted,
    RecordFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStartCommand {
    pub session_id: String,
    pub action: SessionStartAction,
    pub plan: SessionStartPlan,
    pub failure_stage: Option<String>,
    pub failure_error: Option<String>,
}

pub struct SessionStartOutcome {
    pub command: Option<SessionStartCommand>,
    pub result: TransactionResult<SessionStartCommand>,
}

pub struct SessionStartPreview {
    pub labels: NodeLabels,
    pub result: TransactionResult<SessionStartCommand>,
}

#[derive(Clone)]
pub struct SessionStartReconciler {
    graph: Graph<SessionStartCommand>,
    nodes: BTreeMap<String, SessionNodes>,
    commands: BTreeMap<String, SessionStartCommand>,
    labels: NodeLabels,
    next_seq: u64,
}

impl Default for SessionStartReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStartReconciler {
    pub fn new() -> Self {
        Self {
            graph: Graph::<SessionStartCommand>::new_with_command_type(),
            nodes: BTreeMap::new(),
            commands: BTreeMap::new(),
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

    pub fn drive(&mut self, fact: InputFact) -> GraphResult<SessionStartOutcome> {
        let seq = self.next_seq + 1;
        let result = self.stage(&fact, seq, false)?;
        self.next_seq = seq;
        let command = self.translate(&result);
        Ok(SessionStartOutcome { command, result })
    }

    pub fn preview_fact(&mut self, fact: &InputFact) -> GraphResult<Option<SessionStartPreview>> {
        if fact_session_id(fact).is_none() {
            return Ok(None);
        }
        let labels = self.labels.clone();
        let result = self.stage(fact, self.next_seq + 1, true)?;
        Ok(Some(SessionStartPreview { labels, result }))
    }

    pub fn assert_oracle(&self) -> GraphResult<()> {
        self.graph.assert_incremental_equals_full()?;
        Ok(())
    }

    pub fn explain_session_start(&self, session_id: &str) -> Option<SessionStartWhy> {
        let why = self.graph.why_resource_command(&session_key(session_id))?;
        Some(SessionStartWhy {
            resource_key: key_path(&session_key(session_id)),
            last_kind: format!("{:?}", why.kind),
            cause: self.cause_label(&why.cause),
            input_causes: self.labels.labels_for(&why.input_causes),
        })
    }

    pub fn state_rows(&self) -> Vec<SessionStartStateRow> {
        self.commands
            .values()
            .map(|cmd| SessionStartStateRow {
                session_id: cmd.session_id.clone(),
                action: format!("{:?}", cmd.action),
                channel_h: cmd.plan.row.channel_h.clone(),
                signer_pubkey: cmd.plan.row.agent_pubkey.clone(),
                reassert: cmd.plan.reassert,
            })
            .collect()
    }

    fn stage(
        &mut self,
        fact: &InputFact,
        seq: u64,
        preview: bool,
    ) -> GraphResult<TransactionResult<SessionStartCommand>> {
        let Some(session_id) = fact_session_id(fact) else {
            unreachable!("session-start facts are classified before staging")
        };
        let mut nodes = self.nodes.clone();
        let mut labels = self.labels.clone();
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        let session = ensure_session(&mut tx, &mut labels, &mut nodes, session_id)?;
        stage_fact(&mut tx, &session, fact, seq)?;
        let result = if preview { tx.preview()? } else { tx.commit()? };
        if !preview {
            self.nodes = nodes;
            self.labels = labels;
        }
        Ok(result)
    }

    fn translate(
        &mut self,
        result: &TransactionResult<SessionStartCommand>,
    ) -> Option<SessionStartCommand> {
        let command = result
            .resource_plan
            .commands()
            .iter()
            .find_map(|cmd| match cmd {
                ResourceCommand::Open { command, .. }
                | ResourceCommand::Replace { command, .. }
                | ResourceCommand::Refresh { command, .. } => Some(command.clone()),
                ResourceCommand::Close { .. } => None,
            });
        if let Some(command) = &command {
            self.commands
                .insert(command.session_id.clone(), command.clone());
        }
        command
    }

    fn cause_label(&self, cause: &ResourceCommandCause) -> String {
        match cause {
            ResourceCommandCause::Planner { collection } => self
                .labels
                .label_of(*collection)
                .map(str::to_string)
                .unwrap_or_else(|| format!("node:{}", collection.get())),
            ResourceCommandCause::ScopeClosed { scope } => format!("scope-closed: {}", scope.get()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStartStateRow {
    pub session_id: String,
    pub action: String,
    pub channel_h: String,
    pub signer_pubkey: String,
    pub reassert: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStartWhy {
    pub resource_key: String,
    pub last_kind: String,
    pub cause: String,
    pub input_causes: Vec<String>,
}

pub(crate) fn plan_from_request(req: &SessionStartRequestFact) -> SessionStartPlan {
    let active = !req.already_running;
    SessionStartPlan {
        row: SessionRowIntent {
            harness: req.harness.clone(),
            external_id_kind: req.external_id_kind.clone(),
            external_id: req.external_id.clone(),
            agent_pubkey: req.signer_pubkey.clone(),
            agent_slug: req.agent.clone(),
            channel_h: req.channel_for_upsert.clone(),
            child_pid: req.watch_pid,
            resume_id: req.native_id.clone(),
            now: req.at,
        },
        channel_ready: active.then(|| ChannelReadyIntent {
            channel_h: req.channel_h.clone(),
            work_root: req.work_root.clone(),
            room_parent: req.room_parent.clone(),
            base_pubkey: req.base_pubkey.clone(),
        }),
        admit_pubkey: (active && req.signer_ordinal > 0).then(|| req.signer_pubkey.clone()),
        tmux_pane: req.tmux_pane.clone(),
        ring_doorbell: req.ring_doorbell,
        notify_outbox: active,
        ensure_subscription: active,
        replay_chat: active && req.channel_already_subscribed,
        spawn: active.then(|| EngineStartIntent {
            session_id: req.session_id.clone(),
            channel_h: req.channel_h.clone(),
            rel_cwd: req.rel_cwd.clone(),
            watch_pid: req.watch_pid,
            signer_label: req.signer_label.clone(),
            signer_pubkey: req.signer_pubkey.clone(),
        }),
        emit_tail: active,
        reassert: req.already_running,
    }
}
