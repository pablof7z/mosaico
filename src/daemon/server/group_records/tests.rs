use super::*;
use std::collections::BTreeMap;

fn set<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn snapshot() -> CoverageSnapshot {
    CoverageSnapshot::default()
}

/// The daemon watches groups it already knows AND groups whose relay-signed
/// roster names one of its identities. Only the second half can discover a
/// group nobody enumerated, which is the whole reason the subject leaves exist.
#[test]
fn coverage_carries_both_the_known_ids_and_the_local_identities() {
    let coverage = GroupRecordsCoverage::from_snapshot(&CoverageSnapshot {
        daemon_channels: set(["root"]),
        group_state_channels: set(["cached"]),
        addressed_pubkeys: set(["backend-pk", "session-pk"]),
        sessions: BTreeMap::from([("session-pk".to_string(), set(["joined"]))]),
        ..snapshot()
    });

    assert_eq!(coverage.ids, set(["root", "cached", "joined"]));
    assert_eq!(coverage.subjects, set(["backend-pk", "session-pk"]));
    assert!(coverage.predicate().is_some());
}

/// Archived channels are subtracted here, exactly as the subscription
/// reconciler subtracts them, so the two coverage computations cannot drift
/// into watching different sets.
#[test]
fn archived_channels_are_not_watched() {
    let coverage = GroupRecordsCoverage::from_snapshot(&CoverageSnapshot {
        daemon_channels: set(["live", "old"]),
        group_state_channels: set(["old"]),
        archived_channels: set(["old"]),
        ..snapshot()
    });

    assert_eq!(coverage.ids, set(["live"]));
}

/// Nothing to watch is a real state — a daemon with no channels and no
/// identities — and it must not be spelled as an empty relay filter, which a
/// relay would answer with everything or refuse.
#[test]
fn empty_coverage_yields_no_predicate() {
    let coverage = GroupRecordsCoverage::from_snapshot(&snapshot());
    assert!(coverage.predicate().is_none());
}

/// Identities alone are enough: a daemon holding no cached channel at all still
/// asks "which groups list me", which is how a cold cache repopulates.
#[test]
fn identities_alone_are_watchable() {
    let coverage = GroupRecordsCoverage::from_snapshot(&CoverageSnapshot {
        addressed_pubkeys: set(["backend-pk"]),
        ..snapshot()
    });
    assert!(coverage.ids.is_empty());
    assert!(coverage.predicate().is_some());
}

/// The plan is compared by value, so an unchanged daemon does not churn the
/// live subscription on every reconcile pass.
#[test]
fn equal_snapshots_produce_equal_coverage() {
    let build = || {
        GroupRecordsCoverage::from_snapshot(&CoverageSnapshot {
            daemon_channels: set(["root"]),
            addressed_pubkeys: set(["backend-pk"]),
            ..snapshot()
        })
    };
    assert_eq!(build(), build());
}
