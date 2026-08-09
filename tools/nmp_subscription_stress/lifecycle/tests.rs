use clap::Parser;

use super::*;

#[test]
fn every_small_matrix_shape_and_schedule_satisfies_its_exact_oracle() {
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
    for shape in DemandShape::All.selected() {
        for schedule in LifecycleSchedule::All.selected() {
            run(&args, &workload, &fixture, *shape, *schedule).unwrap_or_else(|error| {
                panic!("{shape:?}/{schedule:?} lifecycle oracle failed: {error:#}")
            });
        }
    }
}
