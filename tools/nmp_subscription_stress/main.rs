mod admission_matrix;
mod args;
mod attribution_churn;
mod core;
mod core_metrics;
mod deadline_race;
mod demand_key_matrix;
mod durable_coverage;
mod execution;
mod facade;
mod failure_artifact;
mod freshness;
mod freshness_matrix;
mod freshness_ownership;
mod later_active_owner;
mod lifecycle;
mod matrix;
mod matrix_oracle;
mod measure;
mod metadata_load;
mod nested_freshness;
mod ownership;
mod profile_control;
mod profile_facade;
mod provenance;
mod read_failure;
mod replaceable_freshness;
mod report;
mod resolver;
mod router;
mod router_metrics;
mod schedule;
mod semantic_coverage;
mod store;
mod withdrawal_matrix;
mod workload;

use anyhow::Result;
use clap::Parser;

use args::{Args, Scenario};
use facade::RetainedMode;
use report::Reporter;
use store::DisposableStore;
use workload::Workload;

fn main() -> Result<()> {
    let args = Args::parse();
    args.validate()?;
    let reporter = Reporter::new(&args)?;
    if args.scenario == Scenario::Matrix {
        failure_artifact::run_seeded_matrix(
            &args,
            failure_artifact::RunIdentity {
                nmp_revision: reporter.nmp_revision(),
                harness_revision: reporter.harness_revision(),
                lock_hash: reporter.lock_hash(),
            },
            || matrix::run(&args, &reporter),
        )?;
        reporter.limitations();
        return Ok(());
    }
    let workload = Workload::new(&args)?;
    let needs_store = args.scenario.includes(Scenario::Facade)
        || args.scenario.includes(Scenario::Store)
        || args.scenario.includes(Scenario::Profiles)
        || args.scenario == Scenario::Freshness;
    let fixture = if needs_store {
        let (fixture, seed_metric) = DisposableStore::seed(&args, &workload)?;
        reporter.metric(&seed_metric);
        Some(fixture)
    } else {
        None
    };

    for &topology in args.topologies() {
        if args.scenario.includes(Scenario::Store) {
            reporter.metric(&store::run_store_queries(
                &args,
                &workload,
                fixture.as_ref().expect("store scenario has a fixture"),
                topology,
            )?);
            for metric in resolver::run_resolver(
                &workload,
                fixture.as_ref().expect("store scenario has a fixture"),
                topology,
            )? {
                reporter.metric(&metric);
            }
            for metric in core::run_core(
                &workload,
                fixture.as_ref().expect("store scenario has a fixture"),
                topology,
            )? {
                reporter.metric(&metric);
            }
        }
        if args.scenario.includes(Scenario::Router) {
            for metric in router::run_router(&args, &workload, topology)? {
                reporter.metric(&metric);
            }
        }
        if args.scenario.includes(Scenario::Facade) {
            for retained_mode in [RetainedMode::CacheOnly, RetainedMode::Live] {
                for metric in
                    facade::run_retained(&args, &workload, topology, None, false, retained_mode)?
                {
                    reporter.metric(&metric);
                }
                for metric in facade::run_retained(
                    &args,
                    &workload,
                    topology,
                    Some(
                        fixture
                            .as_ref()
                            .expect("facade scenario has a fixture")
                            .path(),
                    ),
                    false,
                    retained_mode,
                )? {
                    reporter.metric(&metric);
                }
            }
        } else if args.scenario == Scenario::Consumer {
            for metric in
                facade::run_retained(&args, &workload, topology, None, false, RetainedMode::Live)?
            {
                reporter.metric(&metric);
            }
        }
        if args.scenario.includes(Scenario::Consumer) {
            for metric in
                facade::run_retained(&args, &workload, topology, None, true, RetainedMode::Live)?
            {
                reporter.metric(&metric);
            }
        }
    }

    if args.scenario.includes(Scenario::Store) {
        reporter.metric(&store::run_profile_store_queries(
            &args,
            &workload,
            fixture.as_ref().expect("store scenario has a fixture"),
        )?);
    }
    if args.scenario.includes(Scenario::Profiles) {
        for metric in profile_facade::run_cache_only_burst(
            &args,
            &workload,
            fixture
                .as_ref()
                .expect("profile scenario has a fixture")
                .path(),
        )? {
            reporter.metric(&metric);
        }
        for metric in profile_facade::run_live_cohort(
            &args,
            &workload,
            fixture
                .as_ref()
                .expect("profile scenario has a fixture")
                .path(),
        )? {
            reporter.metric(&metric);
        }
    }
    if args.scenario == Scenario::Freshness {
        for metric in freshness::run(
            &args,
            &workload,
            fixture.as_ref().expect("freshness scenario has a fixture"),
        )? {
            reporter.metric(&metric);
        }
    }
    reporter.limitations();
    Ok(())
}
