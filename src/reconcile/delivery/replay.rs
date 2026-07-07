use std::collections::BTreeMap;

use anyhow::Result;
use trellis_core::{Graph, GraphResult, Transaction};
use trellis_testing::{DataTransactionScript, TrellisHarness};

use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::NodeLabels;
use crate::reconcile::replay::ReplayReport;

use super::model::{ensure_session, stage_scan, SessionNodes};
use super::DeliveryCommand;

struct ReplayState {
    sessions: BTreeMap<String, SessionNodes>,
    labels: NodeLabels,
    seq: u64,
}

impl ReplayState {
    fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            labels: NodeLabels::new(),
            seq: 0,
        }
    }

    fn apply(
        &mut self,
        operation: &InputFact,
        tx: &mut Transaction<'_, DeliveryCommand>,
    ) -> GraphResult<()> {
        let InputFact::DeliveryScan(fact) = operation else {
            return Ok(());
        };
        let nodes = ensure_session(tx, &mut self.labels, &mut self.sessions, &fact.session_id)?;
        self.seq += 1;
        stage_scan(tx, &nodes, fact, self.seq)
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
    ReplayReport::from_harness("delivery", &first, export_trace)
}

fn run(
    script: &DataTransactionScript<InputFact>,
) -> Result<TrellisHarness<Graph<DeliveryCommand>, DeliveryCommand>, trellis_testing::ScenarioError>
{
    let mut state = ReplayState::new();
    TrellisHarness::replay_data(
        Graph::<DeliveryCommand>::new_with_command_type,
        script,
        move |operation, tx| state.apply(operation, tx),
    )
}
