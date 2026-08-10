use super::*;
use clap::Parser;

use crate::args::Args;

#[test]
fn one_failed_read_refuses_without_ownership_and_the_runtime_recovers() {
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
    assert_eq!(metric.counts["typed_refusals"], 1);
    assert_eq!(metric.counts["healthy_reopens"], 1);
    assert_eq!(metric.counts["final_ownership_census"], 0);
}
