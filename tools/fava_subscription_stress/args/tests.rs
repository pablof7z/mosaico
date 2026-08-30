use super::*;

#[test]
fn captured_shape_is_the_default() {
    let args = Args::parse_from(["stress"]);
    assert_eq!(args.scenario, Scenario::Captured);
    assert_eq!((args.retained, args.mailboxes), (207, 180));
    assert_eq!(args.topologies().len(), 2);
    assert_eq!(args.demand_shape.selected().len(), 5);
    assert_eq!(args.lifecycle_schedule.selected().len(), 5);
    assert_eq!(args.matrix_counts, [1, 32, 207, 1_000, 4_096, 10_000]);
    args.validate().unwrap();
}

#[test]
fn large_internal_matrix_is_allowed_but_large_thread_fanout_is_refused() {
    let matrix = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--retained",
        "10000",
        "--mailboxes",
        "10000",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "10000",
    ]);
    matrix.validate().unwrap();

    let consumer = Args::parse_from([
        "stress",
        "--scenario",
        "consumer",
        "--retained",
        "10000",
        "--mailboxes",
        "10000",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "10000",
    ]);
    assert!(consumer.validate().is_err());
}

#[test]
fn matrix_sizes_its_fixture_per_case_instead_of_requiring_default_mailboxes() {
    let matrix = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--matrix-counts",
        "1,8",
        "--corpus-rows",
        "1",
    ]);
    matrix.validate().unwrap();
}
