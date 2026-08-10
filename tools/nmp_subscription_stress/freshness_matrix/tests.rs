use clap::Parser;

use super::*;
use crate::args::Args;

#[test]
fn mixed_freshness_keeps_current_max_age_local_and_stale_or_missing_live() {
    let args = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--retained",
        "10",
        "--mailboxes",
        "10",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "10",
    ]);
    let workload = Workload::new(&args).unwrap();
    let (fixture, _) = DisposableStore::seed(&args, &workload).unwrap();
    assert_eq!(run(&workload, &fixture, 10).unwrap().len(), 5);
}
