use std::collections::{BTreeMap, BTreeSet};

use trellis_core::{DependencyList, GraphResult, Transaction, TransactionResult};

use crate::reconcile::journal::InputFact;
use crate::reconcile::labels::NodeLabels;

use super::keys::{plan_subs, Space, SubCommand, SubKey};
use super::{CoverageSnapshot, SessionNodes, SubscriptionReconciler};

pub struct SubscriptionPreview {
    pub result: TransactionResult<SubCommand>,
    pub labels: NodeLabels,
}

#[derive(Clone, Copy)]
struct Handles {
    global_kinds: trellis_core::InputNode<BTreeSet<u16>>,
    daemon_channels: trellis_core::InputNode<BTreeSet<String>>,
    addressed_pubkeys: trellis_core::InputNode<BTreeSet<String>>,
    archived_channels: trellis_core::InputNode<BTreeSet<String>>,
}

impl SubscriptionReconciler {
    pub fn preview_fact(&mut self, fact: &InputFact) -> GraphResult<Option<SubscriptionPreview>> {
        let InputFact::SubscriptionSync { snapshot, .. } = fact else {
            return Ok(None);
        };
        self.preview_sync(snapshot).map(Some)
    }

    pub fn preview_sync(
        &mut self,
        snapshot: &CoverageSnapshot,
    ) -> GraphResult<SubscriptionPreview> {
        let handles = Handles {
            global_kinds: self.global_kinds,
            daemon_channels: self.daemon_channels,
            addressed_pubkeys: self.addressed_pubkeys,
            archived_channels: self.archived_channels,
        };
        let mut labels = self.labels.clone();
        let mut sessions = self.sessions.clone();
        let mut tx = self.graph.begin_transaction()?;
        stage_sync(&mut tx, handles, &mut labels, &mut sessions, snapshot)?;
        let result = tx.preview()?;
        Ok(SubscriptionPreview { result, labels })
    }
}

fn stage_sync(
    tx: &mut Transaction<'_, SubCommand>,
    handles: Handles,
    labels: &mut NodeLabels,
    sessions: &mut BTreeMap<String, SessionNodes>,
    snapshot: &CoverageSnapshot,
) -> GraphResult<()> {
    tx.set_input(handles.global_kinds, super::required_global_kinds())?;
    tx.set_input(handles.daemon_channels, snapshot.daemon_channels.clone())?;
    tx.set_input(
        handles.addressed_pubkeys,
        snapshot.addressed_pubkeys.clone(),
    )?;
    tx.set_input(
        handles.archived_channels,
        snapshot.archived_channels.clone(),
    )?;

    let departed: Vec<String> = sessions
        .keys()
        .filter(|id| !snapshot.sessions.contains_key(*id))
        .cloned()
        .collect();
    for id in departed {
        if let Some(nodes) = sessions.remove(&id) {
            tx.close_scope(nodes.scope)?;
        }
    }

    for (id, channels) in &snapshot.sessions {
        let live: BTreeSet<String> = channels
            .difference(&snapshot.archived_channels)
            .cloned()
            .collect();
        if let Some(nodes) = sessions.get(id) {
            tx.set_input(nodes.channels, live)?;
        } else {
            let scope = tx.create_scope(format!("session-{id}"))?;
            let channels_input = tx.input::<BTreeSet<String>>(format!("session-{id}-channels"))?;
            labels.record(
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
            labels.record(coll.id(), format!("subscriptions/session/{id}/subs"));
            tx.set_resource_planner(coll, scope, plan_subs)?;
            sessions.insert(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_sync_does_not_mutate_subscription_graph_or_labels() {
        let mut r = SubscriptionReconciler::new().unwrap();
        let before_rev = r.revision();
        let before_labels = r.labels().len();
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));

        let preview = r
            .preview_sync(&CoverageSnapshot {
                daemon_channels: BTreeSet::from(["room".to_string()]),
                addressed_pubkeys: BTreeSet::new(),
                archived_channels: BTreeSet::new(),
                sessions,
            })
            .unwrap();

        assert_eq!(r.revision(), before_rev);
        assert_eq!(r.labels().len(), before_labels);
        assert!(preview
            .labels
            .labels_for(&preview.result.changed_inputs)
            .iter()
            .any(|label| label == "subscriptions/session/s1/channels"));
        assert_eq!(preview.result.resource_plan.commands().len(), 3);
    }

    #[test]
    fn preview_sync_matches_next_commit_plan() {
        let mut r = SubscriptionReconciler::new().unwrap();
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
        let snapshot = CoverageSnapshot {
            daemon_channels: BTreeSet::from(["room".to_string()]),
            addressed_pubkeys: BTreeSet::new(),
            archived_channels: BTreeSet::new(),
            sessions,
        };

        let preview = r.preview_sync(&snapshot).unwrap().result;
        let (_effects, committed) = r.sync(&snapshot).unwrap();

        assert_eq!(preview.revision, committed.revision);
        assert!(crate::reconcile::preview::command_plans_match(
            preview.resource_plan.commands(),
            committed.resource_plan.commands()
        ));
    }
}
