use clap::Parser;

use super::*;

#[test]
fn live_and_max_age_profile_opens_report_separate_structural_work() {
    let args = Args::parse_from([
        "stress",
        "--scenario",
        "freshness",
        "--retained",
        "207",
        "--mailboxes",
        "207",
        "--profile-burst",
        "0",
        "--corpus-rows",
        "207",
    ]);
    let workload = Workload::new(&args).unwrap();
    let (fixture, _) = DisposableStore::seed(&args, &workload).unwrap();
    let metrics = run(&args, &workload, &fixture).unwrap();
    let live = &metrics[0];
    let max_age = &metrics[2];
    assert_eq!(live.counts["observations"], 207);
    assert_eq!(live.counts["projection_reads"], 207);
    assert_eq!(live.counts["coverage_reads"], 0);
    assert_eq!(live.counts["wire_ops"], 0);
    assert_eq!(max_age.counts["observations"], 207);
    assert_eq!(max_age.counts["projection_reads"], 207);
    assert_eq!(max_age.counts["coverage_reads"], 207);
    assert_eq!(max_age.counts["wire_ops"], 0);
    eprintln!(
        "freshness_207 live_ms={:.3} max_age_ms={:.3}",
        live.elapsed.as_secs_f64() * 1_000.0,
        max_age.elapsed.as_secs_f64() * 1_000.0
    );
}
