//! The daemon's ONE retained observation of NIP-29 relay-signed group records.
//!
//! Kinds 39000/39001/39002 are watched through a single fava `Observation`.
//! Each fava snapshot is decoded client-side into `Vec<GroupSnapshot>` and
//! pushed into `NmpViews` for all downstream consumers. Coverage filtering
//! happens client-side; no relay-side predicate is used.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;
use fava::{Observation, ObservationClosed};

use super::DaemonState;
use crate::group_snapshot::{self, GroupAvailability, GroupSnapshot};
use crate::reconcile::CoverageSnapshot;

#[path = "group_records/root_names.rs"]
mod root_names;

/// The retained observation and the inputs it was opened for.
#[derive(Default)]
pub(super) struct GroupRecordsWatch {
    /// Reopened only when these change; an unchanged plan keeps the live
    /// subscription rather than churning the relay.
    coverage: Option<GroupRecordsCoverage>,
    /// The drain task owns the Observation; aborting drops it → close().
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

/// Re-point the observation at `coverage`, reopening only if it changed.
pub(in crate::daemon::server) fn sync(
    state: &Arc<DaemonState>,
    coverage: GroupRecordsCoverage,
) -> Result<()> {
    let mut watch = state
        .subscriptions
        .group_records
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if watch.coverage.as_ref() == Some(&coverage) {
        return Ok(());
    }
    // Abort the current drain (drops its Observation → close()) before opening a replacement.
    if let Some(drain) = watch.drain.take() {
        drain.abort();
    }
    if coverage.is_empty() {
        state.nmp().views().set_group_snapshots(Vec::new());
        watch.coverage = Some(coverage);
        return Ok(());
    }
    let observation = state.nmp().open_group_records_observation()?;
    let known_ids = coverage.ids.clone();
    // Observation is moved into the task; aborting the task closes it via Drop.
    watch.drain = Some(tokio::spawn(drain(
        state.clone(),
        observation,
        coverage.clone(),
        known_ids,
    )));
    watch.coverage = Some(coverage);
    Ok(())
}

/// Withdraw the observation (daemon shutdown).
pub(in crate::daemon::server) fn shutdown(state: &DaemonState) {
    let mut watch = state
        .subscriptions
        .group_records
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(drain) = watch.drain.take() {
        drain.abort(); // dropping Observation inside task → close()
    }
    state.nmp().views().set_group_snapshots(Vec::new());
    watch.coverage = None;
}

async fn drain(
    state: Arc<DaemonState>,
    mut observation: Observation,
    coverage: GroupRecordsCoverage,
    mut known_ids: BTreeSet<String>,
) {
    let mut renamed: BTreeSet<String> = BTreeSet::new();
    let mut advertised = managed_roots(&state, &state.nmp().views().group_snapshots());
    loop {
        let fava_snapshot = match observation.changed().await {
            Ok(snapshot) => snapshot,
            Err(ObservationClosed) => return,
        };
        let av = group_snapshot::availability_from_evidence(&fava_snapshot.evidence);
        let all_snapshots =
            group_snapshot::decode_snapshots(&fava_snapshot, &BTreeSet::new(), av);
        let relevant = filter_for_coverage(&all_snapshots, &coverage);
        state.nmp().views().set_group_snapshots(relevant.clone());

        let discovered = record_discoveries(&mut known_ids, &relevant);
        root_names::repair_delivered(&state, &relevant, &mut renamed);
        let managed = managed_roots(&state, &relevant);
        if managed != advertised {
            advertised = managed;
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
fn record_discoveries(
    known_ids: &mut BTreeSet<String>,
    snapshots: &[GroupSnapshot],
) -> bool {
    let mut discovered = false;
    for snapshot in snapshots {
        discovered |= known_ids.insert(snapshot.id.clone());
    }
    discovered
}

fn managed_roots(state: &DaemonState, snapshots: &[GroupSnapshot]) -> Vec<String> {
    let Some(management_pubkey) = state.backend_pubkey() else {
        return Vec::new();
    };
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
