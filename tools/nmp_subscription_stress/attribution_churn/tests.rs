use clap::Parser;

use super::*;

#[test]
fn two_profile_waves_prune_to_one_standing_owner_then_zero() {
    let args = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--retained",
        "9",
        "--mailboxes",
        "9",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "9",
    ]);
    let workload = Workload::new(&args).unwrap();
    let (fixture, _) = DisposableStore::seed(&args, &workload).unwrap();

    assert_eq!(run(&args, &workload, &fixture).unwrap().len(), 8);
}
