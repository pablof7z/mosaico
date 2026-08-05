//! The daemon's ONE retained observation of NIP-29's relay-signed group records.
//!
//! Kinds 39000/39001/39002 are read through a single
//! [`GroupObservation`](nmp::nip29::GroupObservation) that stays open and keeps
//! `relay_channels` / `relay_channel_members` current. Nothing asks the relay
//! for a roster on demand: every consumer reads the cache this drain writes.
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
use crate::fabric::nip29::materializer::Nip29Materializer;
use crate::reconcile::CoverageSnapshot;

#[path = "group_records/root_names.rs"]
mod root_names;

/// The retained observation, and the inputs it was opened for.
#[derive(Default)]
pub(super) struct GroupRecordsWatch {
    /// Reopened only when these change; an unchanged plan keeps the live
    /// subscription rather than churning the relay.
    coverage: Option<GroupRecordsCoverage>,
    /// Aborting the drain drops the observation it owns, and dropping the
    /// observation withdraws the demand. NMP retains nothing keyed by group on
    /// the daemon's behalf, so this handle is the whole lifetime.
    drain: Option<tokio::task::JoinHandle<()>>,
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
    pub(super) fn from_snapshot(snapshot: &CoverageSnapshot) -> Self {
        let mut ids: BTreeSet<String> = BTreeSet::new();
        ids.extend(snapshot.daemon_channels.iter().cloned());
        ids.extend(snapshot.group_state_channels.iter().cloned());
        for channels in snapshot.sessions.values() {
            ids.extend(channels.iter().cloned());
        }
        for archived in &snapshot.archived_channels {
            ids.remove(archived);
        }
        Self {
            subjects: snapshot.addressed_pubkeys.clone(),
            ids,
        }
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
        if let Some(drain) = watch.drain.take() {
            drain.abort();
        }
        watch.coverage = Some(coverage);
        return Ok(());
    };
    // Open the replacement BEFORE withdrawing the old one, so a failure to
    // open leaves the daemon watching what it was already watching rather than
    // watching nothing.
    let observation = state.nmp.observe_group_records(predicate)?;
    if let Some(drain) = watch.drain.take() {
        drain.abort();
    }
    watch.drain = Some(tokio::spawn(drain(state.clone(), observation)));
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
        drain.abort();
    }
    watch.coverage = None;
}

/// Fold every delivery into the cache.
///
/// Each delivery is a COMPLETE snapshot per matching group, never a delta, so
/// a lost or redelivered frame is benign and there is no accumulated state
/// here to corrupt.
async fn drain(state: Arc<DaemonState>, observation: GroupObservation) {
    let mut renamed: BTreeSet<String> = BTreeSet::new();
    let mut advertised = managed_roots(&state);
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
        let delivered: BTreeSet<String> = snapshots
            .iter()
            .map(|snapshot| snapshot.id.clone())
            .collect();
        let discovered = state.with_store(|store| {
            let mut discovered = false;
            for snapshot in &snapshots {
                discovered |= store.get_channel(&snapshot.id).ok().flatten().is_none();
                Nip29Materializer::materialize_group_snapshot(store, snapshot);
            }
            discovered
        });
        // The delivery just moved the relay-signed admin lists, so it may have
        // moved the answer to "which roots does my management key administer?"
        // — both the names this backend owes those roots and the set it
        // advertises. The profile is republished only when that set actually
        // changed; a roster event about a group already advertised is not news.
        root_names::repair_delivered(&state, &delivered, &mut renamed);
        let managed = managed_roots(&state);
        if managed != advertised {
            advertised = managed;
            state.schedule_backend_profile_refresh();
        }
        if discovered {
            // A group nobody had enumerated now names a local identity in its
            // relay-signed roster. Recomputing coverage brings its contents
            // into scope too — and pins the id, so it stays watched even if a
            // later roster drops us.
            let state = state.clone();
            tokio::spawn(async move {
                super::subscriptions::reconcile_subs_logged(&state, "group records discovery")
                    .await;
            });
        }
    }
}

/// The roots this backend currently administers, as the profile would state
/// them. Read here so the drain can tell a delivery that changed the answer
/// from one that merely repeated it.
fn managed_roots(state: &DaemonState) -> Vec<String> {
    let Some(management_pubkey) = state.backend_pubkey() else {
        return Vec::new();
    };
    state
        .with_store(|store| super::backend_profile::managed_roots(store, &management_pubkey))
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "group_records/tests.rs"]
mod tests;
