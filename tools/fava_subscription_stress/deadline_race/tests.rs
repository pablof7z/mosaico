use super::*;
use clap::Parser;

use crate::args::Args;

#[test]
fn a_due_expiration_runs_before_a_simultaneously_ready_command() {
    let args = Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--retained",
        "1",
        "--mailboxes",
        "1",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "1",
    ]);
    let workload = Workload::new(&args).unwrap();
    let metric = run(&workload).unwrap();
    assert_eq!(metric.status, "contract_pass");
    assert_eq!(metric.counts["expired_rows_before_command"], 1);
    assert_eq!(metric.counts["final_ownership_census"], 0);
}
