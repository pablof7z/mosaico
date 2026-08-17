//! The daemon's ONE retained observation of NIP-29's relay-signed group records.
//!
//! Kinds 39000/39001/39002 are read through a single
//! [`GroupObservation`](nmp::nip29::GroupObservation) that stays open and keeps
//! its own folded group state current. Nothing asks the relay for a roster on
//! demand and Mosaico retains no copy: consumers read the observation itself.
//!
//! Branches scale with HOSTS, not groups — one relay is one branch however many
//! groups the predicate names. The honest limit is at the wire, not here: the
//! literal-id leaf lowers to a `#d` set, and a relay may refuse or truncate a
//! filter carrying very many values. A daemon watching very many groups at once
//! would have to shard across several observations; that is not hidden, and it
//! is not done for you.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;
use nmp::nip29::{GroupIds, GroupObservation, GroupPredicate};
use nmp::Binding;

use super::DaemonState;
use crate::reconcile::CoverageSnapshot;

#[path = "group_records/handoff.rs"]
mod handoff;
#[path = "group_records/root_names.rs"]
mod root_names;

/// The retained observation, and the inputs it was opened for.
#[derive(Default)]
pub(super) struct GroupRecordsWatch {
    /// Reopened only when these change; an unchanged plan keeps the live
    /// subscription rather than churning the relay.
    coverage: Option<GroupRecordsCoverage>,
    /// The observation currently exposed through [`NmpViews`](crate::nmp_views::NmpViews).
    /// It stays live while a broader replacement establishes its own state.
    published_observation: Option<Arc<GroupObservation>>,
    /// A replacement acquiring in parallel. It is not visible to consumers
    /// until the handoff barrier proves it covers the still-required groups.
    candidate_observation: Option<Arc<GroupObservation>>,
    /// Delivery owner for `published_observation`.
    published_drain: Option<tokio::task::JoinHandle<()>>,
    /// Delivery owner for `candidate_observation`.
    candidate_drain: Option<tokio::task::JoinHandle<()>>,
}

/// What the daemon wants group records FOR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct GroupRecordsCoverage {
    /// Every identity this daemon speaks for. A group whose relay-signed
    /// roster names one of them is a group this daemon is in — whether or not
    /// it had ever heard of the group. That is discovery, and it is the half
    /// an id-only watch cannot do.
    subjects: BTreeSet<String>,
    /// The groups already known, watched whether or not any local identity is
    /// currently listed in them.
    ids: BTreeSet<String>,
}

impl GroupRecordsCoverage {
    /// Derive the plan from the same coverage snapshot the subscription
    /// reconciler uses, so there is one computation of "which groups" and one
    /// archived-channel subtraction, not two.
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
        // Trusted operators discover groups through relay-signed admin records,
        // but stay separate from addressed daemon identities: their mentions
        // are never routed to the backend.
        subjects.extend(trusted_operators.iter().cloned());
        Self { subjects, ids }
    }

    /// The predicate, or `None` when there is nothing to watch at all.
    ///
    /// Composed, not chosen between: "groups whose member list names one of my
    /// identities, OR whose admin list does, OR that I already know about".
    /// Admin and member are separate leaves because 39001 and 39002 are
    /// separate records — an identity listed only as an admin is not in the
    /// member list, and asking only about members would miss the group this
    /// daemon manages.
    fn predicate(&self) -> Option<GroupPredicate> {
        let mut leaves: Vec<GroupIds> = Vec::new();
        if !self.subjects.is_empty() {
            let subjects = Binding::Literal(self.subjects.clone());
            leaves.push(nmp::nip29::member_list_includes(subjects.clone()));
            leaves.push(nmp::nip29::admin_list_includes(subjects));
        }
        if !self.ids.is_empty() {
            leaves.push(nmp::nip29::any_of(Binding::Literal(self.ids.clone())));
        }
        let mut leaves = leaves.into_iter();
        let first = leaves.next()?;
        Some(first.union(leaves).into())
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
    let Some(predicate) = coverage.predicate() else {
        handoff::cancel_all(&mut watch);
        state.snapshot().nmp.views().set_group_observation(None);
        watch.coverage = Some(coverage);
        return Ok(());
    };
    // Open the replacement BEFORE withdrawing the old one, so a failure to
    // open leaves the daemon watching what it was already watching rather than
    // watching nothing.
    let replacement = Arc::new(state.snapshot().nmp.observe_group_records(predicate)?);
    let known_ids = coverage.ids.clone();
    handoff::cancel_candidate(&mut watch);
    if watch.published_observation.is_none() {
        state
            .snapshot()
            .nmp
            .views()
            .set_group_observation(Some(replacement.clone()));
        watch.published_observation = Some(replacement.clone());
        watch.published_drain = Some(tokio::spawn(drain(
            state.clone(),
            replacement,
            known_ids,
            None,
        )));
    } else {
        watch.candidate_observation = Some(replacement.clone());
        watch.candidate_drain = Some(tokio::spawn(drain(
            state.clone(),
            replacement,
            known_ids,
            Some(coverage.clone()),
        )));
    }
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
    handoff::cancel_all(&mut watch);
    state.snapshot().nmp.views().set_group_observation(None);
    watch.coverage = None;
}

/// React to complete deliveries without retaining a second group-state copy.
///
/// Each delivery is a COMPLETE snapshot per matching group, never a delta, so
/// a lost or redelivered frame is benign and there is no accumulated state
/// here to corrupt.
async fn drain(
    state: Arc<DaemonState>,
    observation: Arc<GroupObservation>,
    mut known_ids: BTreeSet<String>,
    handoff_coverage: Option<GroupRecordsCoverage>,
) {
    let mut renamed: BTreeSet<String> = BTreeSet::new();
    let mut activated = handoff_coverage.is_none();
    let mut advertised = managed_roots(&state, &state.snapshot().nmp.views().group_snapshots());
    loop {
        let snapshots = match observation.next().await {
            Ok(Some(snapshots)) => snapshots,
            Ok(None) => return,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "group records drain: concurrent next — observation abandoned"
                );
                return;
            }
        };
        if !handoff::is_tracked(&state, &observation) {
            return;
        }
        if !activated {
            let Some(coverage) = handoff_coverage.as_ref() else {
                return;
            };
            let Some(prior_snapshots) =
                handoff::activate_if_ready(&state, &observation, coverage, &snapshots)
            else {
                continue;
            };
            advertised = managed_roots(&state, &prior_snapshots);
            activated = true;
        }
        let discovered = record_discoveries(&mut known_ids, &snapshots);
        // The delivery just moved the relay-signed admin lists, so it may have
        // moved the answer to "which roots does my management key administer?"
        // — both the names this backend owes those roots and the set it
        // advertises. The profile is republished only when that set actually
        // changed; a roster event about a group already advertised is not news.
        root_names::repair_delivered(&state, &snapshots, &mut renamed);
        let managed = managed_roots(&state, &snapshots);
        if managed != advertised {
            advertised = managed;
            state.schedule_backend_profile_refresh();
        }
        // Every COMPLETE delivery may change profile demand for a pinned group.
        // Recompute through the idempotent reconciler; the NMP delivery remains
        // the sole roster authority and Mosaico retains no copy.
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

/// Extend the pinned-id set with every group this complete delivery discovered.
fn record_discoveries(
    known_ids: &mut BTreeSet<String>,
    snapshots: &[nmp::nip29::GroupSnapshot],
) -> bool {
    let mut discovered = false;
    for snapshot in snapshots {
        discovered |= known_ids.insert(snapshot.id.clone());
    }
    discovered
}

/// The roots this backend currently administers, as the profile would state
/// them. Read here so the drain can tell a delivery that changed the answer
/// from one that merely repeated it.
fn managed_roots(state: &DaemonState, snapshots: &[nmp::nip29::GroupSnapshot]) -> Vec<String> {
    let Some(management_pubkey) = state.backend_pubkey() else {
        return Vec::new();
    };
    managed_roots_from_snapshots(snapshots, &management_pubkey)
}

/// Root groups whose current NMP snapshot names the backend as an admin.
fn managed_roots_from_snapshots(
    snapshots: &[nmp::nip29::GroupSnapshot],
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
fn is_root(snapshot: &nmp::nip29::GroupSnapshot) -> bool {
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
