use clap::Parser;

use super::*;

fn fixture() -> (Args, Workload, DisposableStore) {
    let args = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--retained",
        "8",
        "--mailboxes",
        "8",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "8",
    ]);
    let workload = Workload::new(&args).unwrap();
    let (fixture, _) = DisposableStore::seed(&args, &workload).unwrap();
    (args, workload, fixture)
}

#[test]
fn duplicate_and_reattach_oracles_cover_each_owner_boundary() {
    let (args, workload, fixture) = fixture();
    assert_eq!(
        run_duplicate_withdrawal(&args, &workload, &fixture)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        run_detached_reattach(&args, &workload, &fixture)
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn partial_pending_cancellation_covers_tags_and_real_avatar_authors() {
    let (args, workload, fixture) = fixture();
    for shape in [DemandShape::CompatibleDistinct, DemandShape::ProfileAuthors] {
        assert_eq!(
            run_partial_pending_cancellation(&args, &workload, &fixture, shape)
                .unwrap()
                .len(),
            9
        );
    }
}
