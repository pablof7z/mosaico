use super::*;
use std::collections::BTreeMap;

use nmp::nip29::{GroupAvailability, GroupMetadata, GroupSnapshot, ListedSubject};
use nmp::RelayUrl;
use nostr::{EventId, Keys, Timestamp};

fn set<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn snapshot() -> CoverageSnapshot {
    CoverageSnapshot::default()
}

fn group_snapshot(
    id: &str,
    parent: Option<&str>,
    about: Option<&str>,
    admin: Option<nostr::PublicKey>,
) -> GroupSnapshot {
    let host = RelayUrl::parse("wss://relay.example").unwrap();
    let mut tags = Vec::new();
    if let Some(parent) = parent {
        tags.push(vec!["parent".to_string(), parent.to_string()]);
    }
    GroupSnapshot {
        id: id.to_string(),
        metadata: Some(GroupMetadata {
            name: Some(id.to_string()),
            about: about.map(str::to_string),
            picture: None,
            tags,
            as_of: Timestamp::from(1),
            event_id: EventId::all_zeros(),
            host: host.clone(),
        }),
        admins: admin
            .map(|pubkey| ListedSubject {
                pubkey,
                role: Some("admin".to_string()),
                hosts: BTreeSet::from([host]),
            })
            .into_iter()
            .collect(),
        members: Vec::new(),
        availability: GroupAvailability::Ready,
        per_host: BTreeMap::new(),
        disagreements: BTreeSet::new(),
    }
}

/// The daemon watches groups it already knows AND groups whose relay-signed
/// roster names one of its identities. Only the second half can discover a
/// group nobody enumerated, which is the whole reason the subject leaves exist.
#[test]
fn coverage_carries_both_the_known_ids_and_the_local_identities() {
    let coverage = GroupRecordsCoverage::from_snapshot(
        &CoverageSnapshot {
            daemon_channels: set(["root"]),
            group_state_channels: set(["cached"]),
            addressed_pubkeys: set(["backend-pk", "session-pk"]),
            sessions: BTreeMap::from([("session-pk".to_string(), set(["joined"]))]),
            ..snapshot()
        },
        &[],
    );

    assert_eq!(coverage.ids, set(["root", "cached", "joined"]));
    assert_eq!(coverage.subjects, set(["backend-pk", "session-pk"]));
    assert!(coverage.predicate().is_some());
}

/// Archived channels are subtracted here, exactly as the subscription
/// reconciler subtracts them, so the two coverage computations cannot drift
/// into watching different sets.
#[test]
fn archived_channels_are_not_watched() {
    let coverage = GroupRecordsCoverage::from_snapshot(
        &CoverageSnapshot {
            daemon_channels: set(["live", "old"]),
            group_state_channels: set(["old"]),
            archived_channels: set(["old"]),
            ..snapshot()
        },
        &[],
    );

    assert_eq!(coverage.ids, set(["live"]));
}

/// Nothing to watch is a real state — a daemon with no channels and no
/// identities — and it must not be spelled as an empty relay filter, which a
/// relay would answer with everything or refuse.
#[test]
fn empty_coverage_yields_no_predicate() {
    let coverage = GroupRecordsCoverage::from_snapshot(&snapshot(), &[]);
    assert!(coverage.predicate().is_none());
}

/// Identities alone are enough: a daemon holding no cached channel at all still
/// asks "which groups list me", which is how a cold cache repopulates.
#[test]
fn identities_alone_are_watchable() {
    let coverage = GroupRecordsCoverage::from_snapshot(
        &CoverageSnapshot {
            addressed_pubkeys: set(["backend-pk"]),
            ..snapshot()
        },
        &[],
    );
    assert!(coverage.ids.is_empty());
    assert!(coverage.predicate().is_some());
}

/// The plan is compared by value, so an unchanged daemon does not churn the
/// live subscription on every reconcile pass.
#[test]
fn equal_snapshots_produce_equal_coverage() {
    let build = || {
        GroupRecordsCoverage::from_snapshot(
            &CoverageSnapshot {
                daemon_channels: set(["root"]),
                addressed_pubkeys: set(["backend-pk"]),
                ..snapshot()
            },
            &[],
        )
    };
    assert_eq!(build(), build());
}

#[test]
fn trusted_operators_are_group_discovery_subjects_but_not_addressed_identities() {
    let snapshot = CoverageSnapshot {
        addressed_pubkeys: set(["backend-pk"]),
        ..snapshot()
    };
    let coverage = GroupRecordsCoverage::from_snapshot(&snapshot, &["operator-pk".to_string()]);

    assert_eq!(coverage.subjects, set(["backend-pk", "operator-pk"]));
    assert_eq!(snapshot.addressed_pubkeys, set(["backend-pk"]));
}

#[test]
fn retained_group_state_is_the_nmp_observation_itself() {
    let watch = GroupRecordsWatch::default();
    let retained: &Option<Arc<GroupObservation>> = &watch.published_observation;
    assert!(retained.is_none());

    let implementation = include_str!("../group_records.rs");
    assert!(!implementation.contains("replace_groups"));
    assert!(!implementation.contains("views().groups"));
}

#[test]
fn handoff_requires_every_still_pinned_or_still_subject_owned_group() {
    let admin = Keys::generate().public_key();
    let member = Keys::generate().public_key();
    let mut member_group = group_snapshot("member-group", None, None, None);
    member_group.members.push(ListedSubject {
        pubkey: member,
        role: Some("member".to_string()),
        hosts: BTreeSet::from([RelayUrl::parse("wss://relay.example").unwrap()]),
    });
    let snapshots = vec![
        group_snapshot("pinned", None, None, None),
        group_snapshot("admin-group", None, None, Some(admin)),
        member_group,
        group_snapshot("departed", None, None, None),
    ];
    let admin_hex = admin.to_hex();
    let member_hex = member.to_hex();
    let coverage = GroupRecordsCoverage {
        subjects: set([&admin_hex, &member_hex]),
        ids: set(["pinned"]),
    };

    assert_eq!(
        handoff::required_snapshot_ids(&coverage, &snapshots),
        set(["pinned", "admin-group", "member-group"])
    );
}

#[test]
fn handoff_waits_for_established_replacement_rows_not_seed_placeholders() {
    let required = set(["first", "second"]);
    let first = group_snapshot("first", None, None, None);
    let mut second = group_snapshot("second", None, None, None);
    second.availability = GroupAvailability::Acquiring;

    assert!(!handoff::establishes(
        &required,
        &[first.clone(), second.clone()]
    ));
    second.availability = GroupAvailability::CachedOnly;
    assert!(handoff::establishes(&required, &[first, second]));
}

#[test]
fn every_new_snapshot_id_is_retained_for_discovery_reconcile() {
    let mut known = set(["known"]);
    let snapshots = vec![
        group_snapshot("first", None, None, None),
        group_snapshot("second", None, None, None),
    ];

    assert!(record_discoveries(&mut known, &snapshots));
    assert_eq!(known, set(["known", "first", "second"]));
    assert!(!record_discoveries(&mut known, &snapshots));
}

#[test]
fn managed_roots_are_derived_from_current_nmp_snapshots() {
    let backend = Keys::generate().public_key();
    let other = Keys::generate().public_key();
    let snapshots = vec![
        group_snapshot("managed", None, None, Some(backend)),
        group_snapshot("child", Some("managed"), None, Some(backend)),
        group_snapshot("archived", None, Some("[ARCHIVED]"), Some(backend)),
        group_snapshot("remote", None, None, Some(other)),
    ];

    assert_eq!(
        managed_roots_from_snapshots(&snapshots, &backend.to_hex()),
        ["managed"]
    );
}
