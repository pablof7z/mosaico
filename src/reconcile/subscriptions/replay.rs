use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use trellis_core::{DependencyList, Graph, GraphResult, Transaction};
use trellis_testing::{DataTransactionScript, ScenarioTarget, TrellisHarness};

use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::NodeLabels;
use crate::reconcile::replay::ReplayReport;

use super::keys::{plan_subs, Space, SubCommand, SubKey};
use super::{CoverageSnapshot, SessionNodes, SubscriptionReconciler};

impl ScenarioTarget<SubCommand> for SubscriptionReconciler {
    fn graph(&self) -> &Graph<SubCommand> {
        &self.graph
    }

    fn graph_mut(&mut self) -> &mut Graph<SubCommand> {
        &mut self.graph
    }
}

struct ReplayState {
    daemon_channels: trellis_core::InputNode<BTreeSet<String>>,
    addressed_pubkeys: trellis_core::InputNode<BTreeSet<String>>,
    archived_channels: trellis_core::InputNode<BTreeSet<String>>,
    sessions: BTreeMap<String, SessionNodes>,
    labels: NodeLabels,
}

impl ReplayState {
    fn from_seed(seed: &SubscriptionReconciler) -> Self {
        Self {
            daemon_channels: seed.daemon_channels,
            addressed_pubkeys: seed.addressed_pubkeys,
            archived_channels: seed.archived_channels,
            sessions: BTreeMap::new(),
            labels: NodeLabels::new(),
        }
    }

    fn apply(
        &mut self,
        operation: &InputFact,
        tx: &mut Transaction<'_, SubCommand>,
    ) -> GraphResult<()> {
        let InputFact::SubscriptionSync { snapshot, .. } = operation else {
            return Ok(());
        };
        self.stage_sync(tx, snapshot)
    }

    fn stage_sync(
        &mut self,
        tx: &mut Transaction<'_, SubCommand>,
        snapshot: &CoverageSnapshot,
    ) -> GraphResult<()> {
        tx.set_input(self.daemon_channels, snapshot.daemon_channels.clone())?;
        tx.set_input(self.addressed_pubkeys, snapshot.addressed_pubkeys.clone())?;
        tx.set_input(self.archived_channels, snapshot.archived_channels.clone())?;

        let departed: Vec<String> = self
            .sessions
            .keys()
            .filter(|id| !snapshot.sessions.contains_key(*id))
            .cloned()
            .collect();
        for id in departed {
            if let Some(nodes) = self.sessions.remove(&id) {
                tx.close_scope(nodes.scope)?;
            }
        }

        for (id, channels) in &snapshot.sessions {
            let live: BTreeSet<String> = channels
                .difference(&snapshot.archived_channels)
                .cloned()
                .collect();
            if let Some(nodes) = self.sessions.get(id) {
                tx.set_input(nodes.channels, live)?;
            } else {
                let scope = tx.create_scope(format!("session-{id}"))?;
                let channels_input =
                    tx.input::<BTreeSet<String>>(format!("session-{id}-channels"))?;
                self.labels.record(
                    channels_input.id(),
                    format!("subscriptions/session/{id}/channels"),
                );
                tx.set_input(channels_input, live)?;
                let coll = tx.set_collection::<SubKey>(
                    format!("session-{id}-subs"),
                    DependencyList::new([channels_input.id()])?,
                    move |ctx| {
                        let mut out = BTreeSet::new();
                        for ch in ctx.input(channels_input)? {
                            out.insert((Space::ChannelH, ch.clone()));
                            out.insert((Space::GroupStateD, ch.clone()));
                        }
                        Ok(out)
                    },
                )?;
                self.labels
                    .record(coll.id(), format!("subscriptions/session/{id}/subs"));
                tx.set_resource_planner(coll, scope, plan_subs)?;
                self.sessions.insert(
                    id.clone(),
                    SessionNodes {
                        scope,
                        channels: channels_input,
                    },
                );
            }
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
    ReplayReport::from_harness("subscriptions", &first, export_trace)
}

fn run(
    script: &DataTransactionScript<InputFact>,
) -> Result<TrellisHarness<SubscriptionReconciler, SubCommand>, trellis_testing::ScenarioError> {
    let seed = SubscriptionReconciler::new().expect("subscription replay seed");
    let mut replay_state = ReplayState::from_seed(&seed);
    drop(seed);
    TrellisHarness::replay_data(
        || SubscriptionReconciler::new().expect("subscription replay target"),
        script,
        move |operation, tx| replay_state.apply(operation, tx),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_capsule_replays_independently() {
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
        let mut script = DataTransactionScript::new();
        script
            .step("sync")
            .operation(InputFact::SubscriptionSync {
                snapshot: CoverageSnapshot {
                    daemon_channels: BTreeSet::from(["room".to_string()]),
                    addressed_pubkeys: BTreeSet::new(),
                    archived_channels: BTreeSet::new(),
                    sessions,
                },
                at: 100,
            })
            .commit();

        let report = replay_script(&script, true).unwrap();
        assert_eq!(report.surface, "subscriptions");
        assert_eq!(report.steps, 1);
        assert!(report.resource_commands >= 2);
        assert!(report.trace_json.is_some());
    }
}
