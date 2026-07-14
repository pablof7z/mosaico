use trellis_core::{GraphResult, Transaction, TransactionResult};

use crate::reconcile::journal::{InputFact, StatusDrive};
use crate::reconcile::labels::NodeLabels;

use super::model::{opts, stage_session, SessionNodes, StaticInfo};
use super::{end_arm, StatusCommand, StatusReconciler};

pub struct StatusPreview {
    pub result: TransactionResult<StatusCommand>,
    pub labels: NodeLabels,
}

impl StatusReconciler {
    pub fn preview_fact(&mut self, fact: &InputFact) -> GraphResult<Option<StatusPreview>> {
        let Some(drive) = status_drive_from_fact(fact) else {
            return Ok(None);
        };
        self.preview_drive(&drive).map(Some)
    }

    pub fn preview_drive(&mut self, drive: &StatusDrive) -> GraphResult<StatusPreview> {
        let sessions = self.sessions.clone();
        let refresh_secs = self.refresh_secs;
        let mut labels = self.labels.clone();
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        stage_drive(&sessions, &mut labels, refresh_secs, drive, &mut tx)?;
        let result = tx.preview()?;
        Ok(StatusPreview { result, labels })
    }
}

pub(super) fn status_drive_from_fact(fact: &InputFact) -> Option<StatusDrive> {
    match fact {
        InputFact::StatusDrive(drive) => Some(drive.clone()),
        InputFact::TurnStarted { pubkey, at } => Some(StatusDrive::TurnStarted {
            pubkey: pubkey.clone(),
            at: *at,
        }),
        InputFact::TurnEnded { pubkey, at } => Some(StatusDrive::TurnEnded {
            pubkey: pubkey.clone(),
            at: *at,
        }),
        InputFact::DistillCompleted {
            pubkey,
            window_hash,
            title,
            activity,
            at,
        } => Some(StatusDrive::DistillCompleted {
            pubkey: pubkey.clone(),
            title: title.clone(),
            activity: activity.clone(),
            window_hash: Some(window_hash.clone()),
            at: *at,
        }),
        _ => None,
    }
}

fn stage_drive(
    sessions: &std::collections::BTreeMap<String, SessionNodes>,
    labels: &mut NodeLabels,
    refresh_secs: u64,
    drive: &StatusDrive,
    tx: &mut Transaction<'_, StatusCommand>,
) -> GraphResult<()> {
    match drive {
        StatusDrive::SessionStarted(args) => {
            if sessions.contains_key(&args.pubkey) {
                return Ok(());
            }
            let info = StaticInfo {
                host: args.host.clone(),
                slug: args.slug.clone(),
                rel_cwd: args.rel_cwd.clone(),
                dispatch_event: args.dispatch_event.clone(),
            };
            stage_session(
                tx,
                labels,
                &args.pubkey,
                info,
                args.channels.clone(),
                args.working,
                args.automatic_delivery,
                &args.title,
                &args.activity,
                args.at / refresh_secs,
            )?;
        }
        StatusDrive::TurnStarted { pubkey, at } => {
            mutate(sessions, refresh_secs, pubkey, *at, tx, |tx, n| {
                tx.set_input(n.working, true)
            })?;
        }
        StatusDrive::TurnEnded { pubkey, at } => {
            mutate(sessions, refresh_secs, pubkey, *at, tx, |tx, n| {
                tx.set_input(n.working, false)
            })?;
        }
        StatusDrive::DistillCompleted {
            pubkey,
            title,
            activity,
            at,
            ..
        } => {
            mutate(sessions, refresh_secs, pubkey, *at, tx, |tx, n| {
                tx.set_input(n.title, title.clone())?;
                tx.set_input(n.activity, activity.clone())
            })?;
        }
        StatusDrive::TitleSet { pubkey, title, at } => {
            mutate(sessions, refresh_secs, pubkey, *at, tx, |tx, n| {
                tx.set_input(n.title, title.clone())
            })?;
        }
        StatusDrive::ChannelsChanged {
            pubkey,
            channels,
            at,
        } => {
            mutate(sessions, refresh_secs, pubkey, *at, tx, |tx, n| {
                tx.set_input(n.channels, channels.clone())
            })?;
        }
        StatusDrive::Tick {
            pubkey,
            automatic_delivery,
            at,
        } => {
            mutate(sessions, refresh_secs, pubkey, *at, tx, |tx, n| {
                tx.set_input(n.automatic_delivery, *automatic_delivery)
            })?;
        }
        StatusDrive::SessionEnded { pubkey, at } => {
            if let Some(nodes) = sessions.get(pubkey) {
                tx.set_input(nodes.live, false)?;
                tx.set_input(nodes.working, false)?;
                tx.set_input(nodes.arm, end_arm(*at, refresh_secs))?;
            }
        }
        StatusDrive::SessionRevoked { pubkey, .. } => {
            if let Some(nodes) = sessions.get(pubkey) {
                tx.close_scope(nodes.scope)?;
            }
        }
    }
    Ok(())
}

fn mutate(
    sessions: &std::collections::BTreeMap<String, SessionNodes>,
    refresh_secs: u64,
    pubkey: &str,
    at: u64,
    tx: &mut Transaction<'_, StatusCommand>,
    stage: impl FnOnce(&mut Transaction<'_, StatusCommand>, &SessionNodes) -> GraphResult<()>,
) -> GraphResult<()> {
    let Some(nodes) = sessions.get(pubkey) else {
        return Ok(());
    };
    stage(tx, nodes)?;
    tx.set_input(nodes.arm, at / refresh_secs)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::reconcile::{InputFact, StatusDrive, StatusSessionStartedArgs};

    use super::*;

    #[test]
    fn preview_fact_does_not_mutate_status_graph_or_labels() {
        let mut r = StatusReconciler::new(90, 30);
        let before_rev = r.revision();
        let before_labels = r.labels().len();
        let fact = InputFact::StatusDrive(StatusDrive::SessionStarted(StatusSessionStartedArgs {
            pubkey: "pk".into(),
            host: "h".into(),
            slug: "a".into(),
            rel_cwd: ".".into(),
            channels: BTreeSet::from(["room".into()]),
            working: true,
            automatic_delivery: true,
            title: "T".into(),
            activity: "reading".into(),
            dispatch_event: None,
            at: 100,
        }));

        let preview = r.preview_fact(&fact).unwrap().unwrap();

        assert_eq!(r.revision(), before_rev);
        assert_eq!(r.labels().len(), before_labels);
        assert_eq!(
            preview.labels.labels_for(&preview.result.changed_inputs),
            vec![
                "status/pk/live",
                "status/pk/working",
                "status/pk/automatic-delivery",
                "status/pk/title",
                "status/pk/activity",
                "status/pk/channels",
                "status/pk/arm",
            ]
        );
        assert_eq!(preview.result.resource_plan.commands().len(), 1);
    }
}
