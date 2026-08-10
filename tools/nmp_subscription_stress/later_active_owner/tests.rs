use clap::Parser;

use super::*;
use crate::args::Args;

#[test]
fn incumbent_eose_settles_the_current_owner_in_both_close_orders() {
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
    assert_eq!(run(&workload, &fixture).unwrap().len(), 8);
}
