use clap::Parser;

use super::*;
use crate::args::Args;

#[test]
fn distinguishes_grouping_uncovered_later_and_exact_residual_contracts() {
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
    let (fixture, _) = DisposableStore::seed(&args, &workload).unwrap();
    let metrics = run(&workload, &fixture).unwrap();

    for phase in ["semantic_pending_grouping", "compatible_later_executes"] {
        let metric = metrics.iter().find(|metric| metric.phase == phase).unwrap();
        assert_eq!(metric.status, "contract_pass");
        assert_eq!(metric.counts["contract_satisfied"], 1);
    }

    for phase in [
        "partial_exact_residual",
        "partial_residual_incumbent_release",
        "partial_residual_final_release",
    ] {
        let metric = metrics.iter().find(|metric| metric.phase == phase).unwrap();
        assert!(matches!(
            metric.status,
            "contract_pass" | "known_red_safe_full_b"
        ));
        assert_eq!(
            metric.counts["contract_satisfied"] + metric.counts["current_safe_full_b"],
            1
        );
    }
}
