use clap::Parser;

use super::*;

#[test]
fn sharding_preserves_values_and_reduces_handles() {
    let args = Args::parse_from([
        "stress",
        "--retained",
        "10",
        "--mailboxes",
        "8",
        "--profile-burst",
        "1",
        "--corpus-rows",
        "10",
        "--shard-size",
        "4",
    ]);
    let workload = Workload::new(&args).unwrap();
    assert_eq!(
        workload
            .retained_queries(Topology::PerIdentity)
            .unwrap()
            .len(),
        10
    );
    assert_eq!(
        workload.retained_queries(Topology::Sharded).unwrap().len(),
        3
    );
    assert_eq!(workload.semantic_values(), 10);
}

#[test]
fn matrix_shapes_preserve_independent_observation_count() {
    let args = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--retained",
        "32",
        "--mailboxes",
        "32",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "32",
    ]);
    let workload = Workload::new(&args).unwrap();
    for shape in DemandShape::All.selected() {
        let queries = workload.matrix_queries(32, *shape).unwrap();
        assert_eq!(queries.len(), 32);
        assert!(queries
            .iter()
            .all(|query| query.branches()[0].freshness == Freshness::Live));
    }
    let duplicates = workload
        .matrix_queries(32, DemandShape::ExactDuplicate)
        .unwrap();
    assert!(duplicates.windows(2).all(|pair| pair[0] == pair[1]));
    let profiles = workload
        .matrix_queries(32, DemandShape::ProfileAuthors)
        .unwrap();
    assert!(profiles.iter().all(|query| {
        let filter = &query.branches()[0].selection;
        filter
            .kinds
            .as_ref()
            .is_some_and(|kinds| kinds.len() == 1 && kinds.contains(&0))
            && filter
                .authors
                .as_ref()
                .is_some_and(|authors| match authors {
                    Binding::Literal(authors) => authors.len() == 1,
                    _ => false,
                })
            && filter.limit.is_none()
    }));
    let limited = workload
        .matrix_queries(32, DemandShape::LimitedIncompatible)
        .unwrap();
    assert!(limited
        .iter()
        .all(|query| query.branches()[0].selection.limit == Some(1)));
    let unlimited = workload
        .matrix_queries(32, DemandShape::UnlimitedMultiAxisIncompatible)
        .unwrap();
    assert!(unlimited.iter().all(|query| {
        let filter = &query.branches()[0].selection;
        filter.limit.is_none() && filter.since.is_some()
    }));
}

#[test]
fn same_coverage_queries_keep_their_window_axes_distinct() {
    let args = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--retained",
        "2",
        "--mailboxes",
        "2",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "2",
    ]);
    let workload = Workload::new(&args).unwrap();
    let queries = workload.demand_key_distinct_queries().unwrap();
    let unbounded = &queries[0].branches()[0].selection;
    let limited = &queries[1].branches()[0].selection;
    assert!(unbounded.limit.is_none() && unbounded.since.is_none() && unbounded.until.is_none());
    assert_eq!(limited.limit, Some(1));
    assert_eq!(limited.since, Some(1_700_000_000));
    assert_eq!(limited.until, Some(1_700_000_100));
}
