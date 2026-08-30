use anyhow::Result;

use crate::admission_matrix;
use crate::args::Args;
use crate::attribution_churn;
use crate::deadline_race;
use crate::demand_key_matrix;
use crate::durable_coverage;
use crate::freshness_matrix;
use crate::freshness_ownership;
use crate::later_active_owner;
use crate::lifecycle;
use crate::metadata_load;
use crate::nested_freshness;
use crate::read_failure;
use crate::replaceable_freshness;
use crate::report::Reporter;
use crate::semantic_coverage;
use crate::store::DisposableStore;
use crate::withdrawal_matrix;
use crate::workload::Workload;

pub(crate) fn run(args: &Args, reporter: &Reporter) -> Result<()> {
    let mut counts = args.matrix_counts.clone();
    counts.sort_unstable();
    counts.dedup();
    let includes_ten_thousand = counts.contains(&10_000);
    let mut ran_exact_oracles = false;
    for count in counts {
        let mut run = args.clone();
        run.retained = count;
        run.mailboxes = count;
        run.profile_burst = 0;
        run.corpus_rows = run.corpus_rows.max(count);
        run.validate()?;
        let workload = Workload::new(&run)?;
        let (fixture, seed) = DisposableStore::seed(&run, &workload)?;
        reporter.metric(&seed);
        for shape in args.demand_shape.selected() {
            for schedule in args.lifecycle_schedule.selected() {
                for metric in lifecycle::run(&run, &workload, &fixture, *shape, *schedule)? {
                    reporter.metric(&metric);
                }
            }
        }
        if args
            .demand_shape
            .selected()
            .contains(&crate::args::DemandShape::ExactDuplicate)
        {
            for metric in admission_matrix::run_active_attach(&run, &workload, &fixture)? {
                reporter.metric(&metric);
            }
            if count > 1 {
                for metric in
                    withdrawal_matrix::run_duplicate_withdrawal(&run, &workload, &fixture)?
                {
                    reporter.metric(&metric);
                }
                for metric in withdrawal_matrix::run_detached_reattach(&run, &workload, &fixture)? {
                    reporter.metric(&metric);
                }
            }
        }
        if count > 1 {
            for shape in [
                crate::args::DemandShape::CompatibleDistinct,
                crate::args::DemandShape::ProfileAuthors,
            ] {
                if args.demand_shape.selected().contains(&shape) {
                    for metric in admission_matrix::run_two_wave(&run, &workload, &fixture, shape)?
                    {
                        reporter.metric(&metric);
                    }
                    for metric in withdrawal_matrix::run_partial_pending_cancellation(
                        &run, &workload, &fixture, shape,
                    )? {
                        reporter.metric(&metric);
                    }
                }
            }
        }
        if count > 2
            && args
                .demand_shape
                .selected()
                .contains(&crate::args::DemandShape::ProfileAuthors)
        {
            for metric in attribution_churn::run(&run, &workload, &fixture)? {
                reporter.metric(&metric);
            }
        }
        if count > 1 {
            for metric in freshness_ownership::run(&workload, &fixture, count / 2)? {
                reporter.metric(&metric);
            }
        }
        if count >= 5 {
            for metric in freshness_matrix::run(&workload, &fixture, count)? {
                reporter.metric(&metric);
            }
        }
        if !ran_exact_oracles {
            for metric in demand_key_matrix::run(&workload, &fixture)? {
                reporter.metric(&metric);
            }
            for metric in nested_freshness::run(&workload, &fixture)? {
                reporter.metric(&metric);
            }
            for metric in later_active_owner::run(&workload, &fixture)? {
                reporter.metric(&metric);
            }
            for metric in replaceable_freshness::run(&workload)? {
                reporter.metric(&metric);
            }
            for metric in durable_coverage::run(&workload)? {
                reporter.metric(&metric);
            }
            reporter.metric(&read_failure::run(&workload)?);
            reporter.metric(&deadline_race::run(&workload)?);
            ran_exact_oracles = true;
        }
    }
    let mut semantic_args = args.clone();
    semantic_args.retained = 3;
    semantic_args.mailboxes = 3;
    semantic_args.profile_burst = 0;
    semantic_args.corpus_rows = semantic_args.corpus_rows.max(3);
    semantic_args.validate()?;
    let semantic_workload = Workload::new(&semantic_args)?;
    let (semantic_fixture, semantic_seed) =
        DisposableStore::seed(&semantic_args, &semantic_workload)?;
    reporter.metric(&semantic_seed);
    for metric in semantic_coverage::run(&semantic_workload, &semantic_fixture)? {
        reporter.metric(&metric);
    }
    for metric in metadata_load::run(includes_ten_thousand)? {
        reporter.metric(&metric);
    }
    Ok(())
}
