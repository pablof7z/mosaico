use super::*;

use crate::group_snapshot::{GroupAvailability, GroupMetadata, GroupSnapshot, ListedSubject};

fn set<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn coverage_snapshot() -> CoverageSnapshot {
    CoverageSnapshot::default()
}

fn group_snapshot(
    id: &str,
    parent: Option<&str>,
    about: Option<&str>,
    admin: Option<nostr::key::PublicKey>,
) -> GroupSnapshot {
    let mut tags = Vec::new();
    if let Some(parent) = parent {
        tags.push(vec!["parent".to_string(), parent.to_string()]);
    }
    GroupSnapshot {
        id: id.to_string(),
        metadata: Some(GroupMetadata {
            name: Some(id.to_string()),
            about: about.map(str::to_string),
            tags,
            as_of: std::time::Duration::from_secs(1),
        }),
        admins: admin
            .map(|pubkey| ListedSubject::new(pubkey, vec!["admin".to_string()]))
            .into_iter()
            .collect(),
        members: Vec::new(),
        availability: GroupAvailability::Ready,
    }
}

/// The daemon watches groups it already knows AND groups whose relay-signed
/// roster names one of its identities.
#[test]
fn coverage_carries_both_the_known_ids_and_the_local_identities() {
    let coverage = GroupRecordsCoverage::from_snapshot(
        &CoverageSnapshot {
            daemon_channels: set(["root"]),
            group_state_channels: set(["cached"]),
            addressed_pubkeys: set(["backend-pk", "session-pk"]),
            sessions: std::collections::BTreeMap::from([(
                "session-pk".to_string(),
                set(["joined"]),
            )]),
            ..coverage_snapshot()
        },
        &[],
    );

    assert_eq!(coverage.ids, set(["root", "cached", "joined"]));
    assert_eq!(coverage.subjects, set(["backend-pk", "session-pk"]));
    assert!(!coverage.is_empty());
}

/// Archived channels are subtracted here, exactly as the subscription
/// reconciler subtracts them.
#[test]
fn archived_channels_are_not_watched() {
    let coverage = GroupRecordsCoverage::from_snapshot(
        &CoverageSnapshot {
            daemon_channels: set(["live", "old"]),
            group_state_channels: set(["old"]),
            archived_channels: set(["old"]),
            ..coverage_snapshot()
        },
        &[],
    );

    assert_eq!(coverage.ids, set(["live"]));
}

/// Nothing to watch is a real state.
#[test]
fn empty_coverage_is_empty() {
    let coverage = GroupRecordsCoverage::from_snapshot(&coverage_snapshot(), &[]);
    assert!(coverage.is_empty());
}

/// Identities alone are enough: a daemon holding no cached channel still
/// wants "which groups list me".
#[test]
fn identities_alone_are_watchable() {
    let coverage = GroupRecordsCoverage::from_snapshot(
        &CoverageSnapshot {
            addressed_pubkeys: set(["backend-pk"]),
            ..coverage_snapshot()
        },
        &[],
    );
    assert!(coverage.ids.is_empty());
    assert!(!coverage.is_empty());
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
                ..coverage_snapshot()
            },
            &[],
        )
    };
    assert_eq!(build(), build());
}

#[test]
fn trusted_operators_are_group_discovery_subjects_but_not_addressed_identities() {
    let snap = CoverageSnapshot {
        addressed_pubkeys: set(["backend-pk"]),
        ..coverage_snapshot()
    };
    let coverage = GroupRecordsCoverage::from_snapshot(&snap, &["operator-pk".to_string()]);

    assert_eq!(coverage.subjects, set(["backend-pk", "operator-pk"]));
    assert_eq!(snap.addressed_pubkeys, set(["backend-pk"]));
}

#[test]
fn filter_keeps_groups_matching_coverage() {
    let admin = nostr::key::Keys::generate().public_key();
    let member = nostr::key::Keys::generate().public_key();
    let mut member_group = group_snapshot("member-group", None, None, None);
    member_group
        .members
        .push(ListedSubject::new(member, Vec::new()));
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

    let filtered = filter_for_coverage(&snapshots, &coverage);
    let ids: BTreeSet<String> = filtered.into_iter().map(|s| s.id).collect();
    assert_eq!(ids, set(["pinned", "admin-group", "member-group"]));
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
fn managed_roots_are_derived_from_current_snapshots() {
    let backend = nostr::key::Keys::generate().public_key();
    let other = nostr::key::Keys::generate().public_key();
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
