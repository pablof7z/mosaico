use clap::Parser;

use super::*;
use crate::args::Args;

#[test]
fn old_new_and_future_replaceable_rows_keep_coverage_time_independent() {
    let args = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--retained",
        "3",
        "--mailboxes",
        "3",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "3",
    ]);
    let workload = Workload::new(&args).unwrap();
    let metrics = run(&workload).unwrap();
    assert_eq!(metrics.len(), 8);
}
