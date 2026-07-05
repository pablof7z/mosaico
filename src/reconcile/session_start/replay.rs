use std::collections::BTreeMap;

use anyhow::Result;
use trellis_core::{Graph, GraphResult, Transaction};
use trellis_testing::{DataTransactionScript, TrellisHarness};

use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::NodeLabels;
use crate::reconcile::replay::ReplayReport;

use super::model::{ensure_session, fact_session_id, stage_fact, SessionNodes};
use super::SessionStartCommand;

struct ReplayState {
    nodes: BTreeMap<String, SessionNodes>,
    labels: NodeLabels,
    seq: u64,
}

impl ReplayState {
    fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            labels: NodeLabels::new(),
            seq: 0,
        }
    }

    fn apply(
        &mut self,
        operation: &InputFact,
        tx: &mut Transaction<'_, SessionStartCommand>,
    ) -> GraphResult<()> {
        let Some(session_id) = fact_session_id(operation) else {
            return Ok(());
        };
        let nodes = ensure_session(tx, &mut self.labels, &mut self.nodes, session_id)?;
        self.seq += 1;
        stage_fact(tx, &nodes, operation, self.seq)
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
    ReplayReport::from_harness("session_start", &first, export_trace)
}

fn run(
    script: &DataTransactionScript<InputFact>,
) -> Result<
    TrellisHarness<Graph<SessionStartCommand>, SessionStartCommand>,
    trellis_testing::ScenarioError,
> {
    let mut state = ReplayState::new();
    TrellisHarness::replay_data(
        Graph::<SessionStartCommand>::new_with_command_type,
        script,
        move |operation, tx| state.apply(operation, tx),
    )
}
