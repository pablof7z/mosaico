//! Lossless ownership transfer between two NMP group observations.
//!
//! Mosaico never merges their rows. It keeps the predecessor live and visible
//! until the replacement's own complete delivery establishes every predecessor
//! group that the new predicate still requires, then swaps the NMP handles.

use std::collections::BTreeSet;
use std::sync::Arc;

use nmp::nip29::{GroupAvailability, GroupObservation, GroupSnapshot};

use super::{DaemonState, GroupRecordsCoverage, GroupRecordsWatch};

pub(super) fn cancel_candidate(watch: &mut GroupRecordsWatch) {
    if let Some(drain) = watch.candidate_drain.take() {
        drain.abort();
    }
    if let Some(observation) = watch.candidate_observation.take() {
        observation.cancel();
    }
}

pub(super) fn cancel_all(watch: &mut GroupRecordsWatch) {
    cancel_candidate(watch);
    if let Some(drain) = watch.published_drain.take() {
        drain.abort();
    }
    if let Some(observation) = watch.published_observation.take() {
        observation.cancel();
    }
}

pub(super) fn is_tracked(state: &DaemonState, observation: &Arc<GroupObservation>) -> bool {
    let watch = state
        .subscriptions
        .group_records
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    is_same(&watch.published_observation, observation)
        || is_same(&watch.candidate_observation, observation)
}

/// Publish `candidate` only after its delivered frame has established every
/// predecessor group the replacement predicate still asks for.
///
/// Returns the predecessor snapshots so the new drain can compare product
/// side effects across the handoff without copying them into retained state.
pub(super) fn activate_if_ready(
    state: &DaemonState,
    candidate: &Arc<GroupObservation>,
    coverage: &GroupRecordsCoverage,
    delivered: &[GroupSnapshot],
) -> Option<Vec<GroupSnapshot>> {
    let mut watch = state
        .subscriptions
        .group_records
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if !is_same(&watch.candidate_observation, candidate) {
        return None;
    }
    let prior = watch
        .published_observation
        .as_ref()
        .map(|observation| observation.latest())
        .unwrap_or_default();
    let required = required_snapshot_ids(coverage, &prior);
    if !establishes(&required, delivered) {
        return None;
    }

    state
        .nmp()
        .views()
        .set_group_observation(Some(candidate.clone()));
    let predecessor = watch.published_observation.replace(candidate.clone());
    watch.candidate_observation = None;
    if let Some(drain) = watch.published_drain.take() {
        drain.abort();
    }
    watch.published_drain = watch.candidate_drain.take();
    if let Some(predecessor) = predecessor {
        predecessor.cancel();
    }
    Some(prior)
}

pub(super) fn required_snapshot_ids(
    coverage: &GroupRecordsCoverage,
    snapshots: &[GroupSnapshot],
) -> BTreeSet<String> {
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
        .map(|snapshot| snapshot.id.clone())
        .collect()
}

pub(super) fn establishes(required: &BTreeSet<String>, delivered: &[GroupSnapshot]) -> bool {
    required.iter().all(|id| {
        delivered.iter().any(|snapshot| {
            snapshot.id == *id
                && matches!(
                    snapshot.availability,
                    GroupAvailability::Ready | GroupAvailability::CachedOnly
                )
        })
    })
}

fn is_same(slot: &Option<Arc<GroupObservation>>, observation: &Arc<GroupObservation>) -> bool {
    slot.as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, observation))
}
