use clap::Parser;

use super::*;
use crate::args::Args;

#[test]
fn live_and_cache_only_same_query_have_independent_observations_and_ownership() {
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
    assert_eq!(run(&workload, &fixture, 4).unwrap().len(), 10);
}
