use clap::Parser;

use super::*;
use crate::args::Args;

#[test]
fn window_distinct_demands_keep_request_evidence_and_teardown_exact() {
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
    let (fixture, _) = DisposableStore::seed(&args, &workload).unwrap();
    assert_eq!(run(&workload, &fixture).unwrap().len(), 5);
}
