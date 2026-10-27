//! The daemon's one stable demand for NIP-29 relay-signed group records.
//!
//! Kinds 39000/39001/39002 are read through one stable SDK demand. Consumers
//! reread the complete decoded view after wake-only notifications.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;

use super::DaemonState;
use crate::group_snapshot::GroupSnapshot;
use crate::reconcile::CoverageSnapshot;

const GROUP_RECORDS_DEMAND: &str = "mosaico-group-records";

#[path = "group_records/root_names.rs"]
mod root_names;

/// The live view task and the inputs its stable demand was opened for.
#[derive(Default)]
pub(super) struct GroupRecordsWatch {
    /// Reopened only when these change; an unchanged plan keeps the live
    /// subscription rather than churning the relay.
    coverage: Option<GroupRecordsCoverage>,
    /// The task waits for complete-view revisions.
    drain: Option<tokio::task::JoinHandle<()>>,
}

/// What the daemon wants group records FOR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct GroupRecordsCoverage {
    /// Every identity this daemon speaks for. A group whose relay-signed
    /// roster names one of them is discovered even without a prior id.
    subjects: BTreeSet<String>,
    /// The groups already known, watched whether or not any local identity is
    /// currently listed in them.
    ids: BTreeSet<String>,
}

impl GroupRecordsCoverage {
    /// Derive the plan from the same coverage snapshot the subscription
    /// reconciler uses.
    pub(super) fn from_snapshot(snapshot: &CoverageSnapshot, trusted_operators: &[String]) -> Self {
        let mut ids: BTreeSet<String> = BTreeSet::new();
        ids.extend(snapshot.daemon_channels.iter().cloned());
        ids.extend(snapshot.group_state_channels.iter().cloned());
        for channels in snapshot.sessions.values() {
            ids.extend(channels.iter().cloned());
        }
        for archived in &snapshot.archived_channels {
            ids.remove(archived);
        }
        let mut subjects = snapshot.addressed_pubkeys.clone();
        subjects.extend(trusted_operators.iter().cloned());
        Self { subjects, ids }
    }

    fn is_empty(&self) -> bool {
        self.subjects.is_empty() && self.ids.is_empty()
    }
}

/// Re-point the stable demand at `coverage`, replacing only if it changed.
pub(in crate::daemon::server) async fn sync(
    state: &Arc<DaemonState>,
    coverage: GroupRecordsCoverage,
) -> Result<()> {
    let prior_drain = {
        let mut watch = state
            .subscriptions
            .group_records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if watch.coverage.as_ref() == Some(&coverage) {
            return Ok(());
        }
        watch.coverage = Some(coverage.clone());
        watch.drain.take()
    };
    if let Some(drain) = prior_drain {
        drain.abort();
    }
    if coverage.is_empty() {
        state
            .nip29_view()
            .replace_group_scope(BTreeSet::new(), BTreeSet::new());
        state.nostr().close_demand(GROUP_RECORDS_DEMAND).await?;
        return Ok(());
    }
    state
        .nip29_view()
        .replace_group_scope(coverage.ids.clone(), coverage.subjects.clone());
    state
        .nostr()
        .replace_demand(
            GROUP_RECORDS_DEMAND,
            vec![nostr::prelude::Filter::new()
                .kinds(
                    [39_000, 39_001, 39_002]
                        .map(nostr::prelude::Kind::Custom)
                        .into_iter()
                        .chain(std::iter::once(nostr::prelude::Kind::EventDeletion)),
                )
                .limit(crate::nostr_runtime::GROUP_RECORD_LIMIT)],
        )
        .await?;
    let known_ids = coverage.ids.clone();
    let drain = tokio::spawn(drain(state.clone(), coverage, known_ids));
    state
        .subscriptions
        .group_records
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .drain = Some(drain);
    Ok(())
}

/// Withdraw the observation (daemon shutdown).
pub(in crate::daemon::server) async fn shutdown(state: &DaemonState) {
    let drain = {
        let mut watch = state
            .subscriptions
            .group_records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        watch.coverage = None;
        watch.drain.take()
    };
    if let Some(drain) = drain {
        drain.abort();
    }
    state
        .nip29_view()
        .replace_group_scope(BTreeSet::new(), BTreeSet::new());
    let _ = state.nostr().close_demand(GROUP_RECORDS_DEMAND).await;
}

async fn drain(
    state: Arc<DaemonState>,
    coverage: GroupRecordsCoverage,
    mut known_ids: BTreeSet<String>,
) {
    let view = state.nip29_view();
    let mut changes = view.subscribe_changes();
    let mut renamed: BTreeSet<String> = BTreeSet::new();
    let mut advertised = managed_roots(&state, view.current().group_snapshots());
    apply_snapshot(
        &state,
        view.current().group_snapshots(),
        &coverage,
        &mut known_ids,
        &mut renamed,
        &mut advertised,
    );
    loop {
        if changes.changed().await.is_err() {
            return;
        }
        apply_snapshot(
            &state,
            view.current().group_snapshots(),
            &coverage,
            &mut known_ids,
            &mut renamed,
            &mut advertised,
        );
    }
}

fn apply_snapshot(
    state: &Arc<DaemonState>,
    snapshots: &[GroupSnapshot],
    coverage: &GroupRecordsCoverage,
    known_ids: &mut BTreeSet<String>,
    renamed: &mut BTreeSet<String>,
    advertised: &mut Vec<String>,
) {
    let relevant = filter_for_coverage(snapshots, coverage);
    let discovered = record_discoveries(known_ids, &relevant);
    root_names::repair_delivered(state, &relevant, renamed);
    let managed = managed_roots(state, &relevant);
    if managed != *advertised {
        *advertised = managed;
        state.schedule_backend_profile_refresh();
    }
    let state = state.clone();
    tokio::spawn(async move {
        let cause = if discovered {
            "group records discovery"
        } else {
            "group records delivery"
        };
        super::subscriptions::reconcile_subs_logged(&state, cause).await;
    });
}

/// Keep only the snapshots this coverage predicate cares about.
fn filter_for_coverage(
    snapshots: &[GroupSnapshot],
    coverage: &GroupRecordsCoverage,
) -> Vec<GroupSnapshot> {
    if coverage.is_empty() {
        return Vec::new();
    }
    snapshots
        .iter()
        .filter(|snapshot| {
            coverage.ids.contains(&snapshot.id)
                || snapshot
                    .admins
                    .iter()
                    .any(|admin| coverage.subjects.contains(&admin.pubkey.to_hex()))
                || snapshot
                    .members
                    .iter()
                    .any(|member| coverage.subjects.contains(&member.pubkey.to_hex()))
        })
        .cloned()
        .collect()
}

/// Extend the pinned-id set with every group this delivery discovered.
fn record_discoveries(known_ids: &mut BTreeSet<String>, snapshots: &[GroupSnapshot]) -> bool {
    let mut discovered = false;
    for snapshot in snapshots {
        discovered |= known_ids.insert(snapshot.id.clone());
    }
    discovered
}

fn managed_roots(state: &DaemonState, snapshots: &[GroupSnapshot]) -> Vec<String> {
    let management_pubkey = state.backend_pubkey();
    managed_roots_from_snapshots(snapshots, &management_pubkey)
}

/// Root groups whose current snapshot names the backend as an admin.
fn managed_roots_from_snapshots(
    snapshots: &[GroupSnapshot],
    management_pubkey: &str,
) -> Vec<String> {
    snapshots
        .iter()
        .filter(|snapshot| is_root(snapshot))
        .filter(|snapshot| {
            snapshot
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.about.as_deref())
                .is_none_or(|about| !crate::state::is_archived_channel_about(about))
        })
        .filter(|snapshot| {
            snapshot
                .admins
                .iter()
                .any(|admin| admin.pubkey.to_hex() == management_pubkey)
        })
        .map(|snapshot| snapshot.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Metadata without a non-empty `parent` row describes a top-level group.
fn is_root(snapshot: &GroupSnapshot) -> bool {
    snapshot.metadata.as_ref().is_some_and(|metadata| {
        metadata
            .tags
            .iter()
            .find(|row| row.first().map(String::as_str) == Some("parent"))
            .and_then(|row| row.get(1))
            .is_none_or(String::is_empty)
    })
}

#[cfg(test)]
#[path = "group_records/tests.rs"]
mod tests;
