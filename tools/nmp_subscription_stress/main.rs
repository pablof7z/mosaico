mod args;
mod core;
mod facade;
mod measure;
mod report;
mod resolver;
mod router;
mod store;
mod workload;

use anyhow::Result;
use clap::Parser;

use args::{Args, Scenario};
use report::Reporter;
use store::DisposableStore;
use workload::Workload;

fn main() -> Result<()> {
    let args = Args::parse();
    args.validate()?;
    let reporter = Reporter::new(&args);
    let workload = Workload::new(&args)?;
    let needs_store = args.scenario.includes(Scenario::Facade)
        || args.scenario.includes(Scenario::Store)
        || args.scenario.includes(Scenario::Profiles);
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
            for metric in facade::run_retained(&args, &workload, topology, None, false)? {
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
            )? {
                reporter.metric(&metric);
            }
        } else if args.scenario == Scenario::Consumer {
            for metric in facade::run_retained(&args, &workload, topology, None, false)? {
                reporter.metric(&metric);
            }
        }
        if args.scenario.includes(Scenario::Consumer) {
            for metric in facade::run_retained(&args, &workload, topology, None, true)? {
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
        for metric in facade::run_profile_burst(
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
    reporter.limitations();
    Ok(())
}
