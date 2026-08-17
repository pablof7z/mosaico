//! Projecting NMP's relay-signed group records into the `relay_*` caches.
//!
//! The whole of kinds 39000/39001/39002 arrives here as one
//! [`GroupSnapshot`] — metadata, admins and members already folded across
//! every host in scope, with per-host attribution beside them. Mosaico never
//! walks a `p` row and never decides what a role-less admin row means; NMP
//! reports the role the relay wrote, or reports that it wrote none.

use super::*;
use crate::fabric::ProjectionProvenance;
use crate::state::ProjectionKind;
use nmp::nip29::GroupSnapshot;

impl Nip29Materializer {
    /// Materialise one kind:39000 into `relay_channels`.
    ///
    /// Kept beside the snapshot projection because the daemon also watches the
    /// UNKEYED kind:39000 feed — every group these relays serve, with no `d`
    /// predicate — and NMP's group-records door takes a predicate by
    /// construction. That feed carries metadata alone, so it has no roster to
    /// fold and needs no snapshot.
    pub(crate) fn materialize_channel(
        store: &Store,
        event: &Event,
        provenance: &ProjectionProvenance,
    ) {
        let Some(channel_h) = super::super::nostr_tag(event, "d") else {
            return;
        };
        let name = super::super::nostr_tag(event, "name").unwrap_or("");
        let about = super::super::nostr_tag(event, "about").unwrap_or("");
        let parent = super::super::nostr_tag(event, "parent").unwrap_or("");
        let projected = store
            .upsert_channel(channel_h, name, about, parent, event.created_at.as_secs())
            .and_then(|()| {
                store.set_projection_source(
                    ProjectionKind::Channel,
                    channel_h,
                    &provenance.source_event_id,
                )
            });
        if let Err(e) = projected {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "materialize_channel: relay_channels upsert failed — relay truth diverged from cache"
            );
        }
    }

    /// Materialise one group's relay-signed records into `relay_channels` and
    /// `relay_channel_members`.
    ///
    /// Each of the three records is written only when some host actually
    /// published it. That distinction is the point: a snapshot for a group
    /// whose hosts have published no kind:39002 carries an empty
    /// `members` vector, and so does a snapshot for a group whose hosts
    /// published an EMPTY kind:39002. The first must not clear the cache and
    /// the second must, so the per-host records — not the folded vector —
    /// decide whether a replacement happens at all.
    pub(crate) fn materialize_group_snapshot(store: &Store, snapshot: &GroupSnapshot) {
        let channel_h = snapshot.id.as_str();

        if let Some(metadata) = &snapshot.metadata {
            let parent = metadata
                .tags
                .iter()
                .find(|row| row.first().map(String::as_str) == Some("parent"))
                .and_then(|row| row.get(1))
                .map(String::as_str)
                .unwrap_or("");
            if let Err(e) = store.upsert_channel(
                channel_h,
                metadata.name.as_deref().unwrap_or(""),
                metadata.about.as_deref().unwrap_or(""),
                parent,
                metadata.as_of.as_secs(),
            ) {
                tracing::error!(
                    channel = channel_h,
                    error = %e,
                    "materialize_group_snapshot: relay_channels upsert failed — relay truth diverged from cache"
                );
            }
        }

        // The high-water mark the store guards replacements with is the newest
        // `created_at` any host stamped on the record. The union itself has no
        // single timestamp — it is several hosts' records — so the newest one
        // is what the replacement is "as of".
        if let Some(as_of) = record_as_of(snapshot, |host| host.admins.as_ref()) {
            let admins = subject_pubkeys(&snapshot.admins);
            if let Err(e) = store.replace_channel_admins(channel_h, &admins, as_of) {
                tracing::error!(
                    channel = channel_h,
                    error = %e,
                    "materialize_group_snapshot: replace_channel_admins failed — relay truth diverged from cache"
                );
            }
        }

        if let Some(as_of) = record_as_of(snapshot, |host| host.members.as_ref()) {
            let members = subject_pubkeys(&snapshot.members);
            if let Err(e) = store.replace_channel_members(channel_h, &members, as_of) {
                tracing::error!(
                    channel = channel_h,
                    error = %e,
                    "materialize_group_snapshot: replace_channel_members failed — relay truth diverged from cache"
                );
            }
        }
    }
}

/// The newest `created_at` any host stamped on the selected record, or `None`
/// when no host in scope has published it.
fn record_as_of(
    snapshot: &GroupSnapshot,
    select: impl Fn(&nmp::nip29::HostRecords) -> Option<&nmp::nip29::ListedRecord>,
) -> Option<u64> {
    snapshot
        .per_host
        .values()
        .filter_map(&select)
        .map(|record| record.as_of.as_secs())
        .max()
}

/// The subjects as the store spells a pubkey: lowercase hex, never bech32.
fn subject_pubkeys(subjects: &[nmp::nip29::ListedSubject]) -> Vec<String> {
    subjects
        .iter()
        .map(|subject| subject.pubkey.to_hex())
        .collect()
}
