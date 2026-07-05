use std::collections::BTreeMap;

use anyhow::Result;
use trellis_core::{Graph, GraphResult, Transaction};
use trellis_testing::{DataTransactionScript, TrellisHarness};

use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::NodeLabels;
use crate::reconcile::replay::ReplayReport;

use super::model::{ensure_entry, fact_seed, stage_fact, EntryNodes};
use super::OutboxCommand;

struct ReplayState {
    nodes: BTreeMap<i64, EntryNodes>,
    retries: BTreeMap<i64, i64>,
    labels: NodeLabels,
    seq: u64,
}

impl ReplayState {
    fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            retries: BTreeMap::new(),
            labels: NodeLabels::new(),
            seq: 0,
        }
    }

    fn apply(
        &mut self,
        operation: &InputFact,
        tx: &mut Transaction<'_, OutboxCommand>,
    ) -> GraphResult<()> {
        let Some(mut seed) = fact_seed(operation) else {
            return Ok(());
        };
        seed.retries = self.retries.get(&seed.local_id).copied().unwrap_or(0);
        let nodes = ensure_entry(tx, &mut self.labels, &mut self.nodes, &seed)?;
        self.seq += 1;
        stage_fact(tx, &nodes, &seed, operation, self.seq)?;
        if let InputFact::RelayPublishAccepted {
            accepted: false, ..
        } = operation
        {
            self.retries.insert(seed.local_id, seed.retries + 1);
        }
        Ok(())
    }
}

pub(crate) fn replay_script(
    script: &DataTransactionScript<InputFact>,
    export_trace: bool,
) -> Result<ReplayReport> {
    let first = run(script).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let second = run(script).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    first
        .assert_replay_matches(&second)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ReplayReport::from_harness("outbox", &first, export_trace)
}

fn run(
    script: &DataTransactionScript<InputFact>,
) -> Result<TrellisHarness<Graph<OutboxCommand>, OutboxCommand>, trellis_testing::ScenarioError> {
    let mut state = ReplayState::new();
    TrellisHarness::replay_data(
        Graph::<OutboxCommand>::new_with_command_type,
        script,
        move |operation, tx| state.apply(operation, tx),
    )
}
