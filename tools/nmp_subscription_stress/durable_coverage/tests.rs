use clap::Parser;

use super::*;
use crate::args::Args;

#[test]
fn nmp_written_coverage_survives_restart_and_expires_by_reducer_time() {
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
    assert_eq!(metrics.len(), 6);
}
