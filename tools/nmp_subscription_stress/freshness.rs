use std::time::Instant;

use anyhow::{Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg};
use nmp::Freshness;
use nmp_store::RedbStore;

use crate::args::Args;
use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::lifecycle::{close_phase, observation_id};
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
) -> Result<Vec<Metric>> {
    let mut metrics = Vec::new();
    metrics.extend(run_mode(args, workload, fixture, Freshness::Live, "live")?);
    metrics.extend(run_mode(
        args,
        workload,
        fixture,
        Freshness::MaxAge { seconds: 3_600 },
        "max_age_missing_coverage",
    )?);
    Ok(metrics)
}

fn run_mode(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
    freshness: Freshness,
    label: &'static str,
) -> Result<Vec<Metric>> {
    let store = RedbStore::open(fixture.path()).context("opening freshness stress store")?;
    let mut core = EngineCore::new(store, 8);
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut effects = EffectCounts::default();
    let mut ids = Vec::with_capacity(args.retained);
    for index in 0..args.retained {
        let query = workload.profile_query_with_freshness(index, freshness)?;
        let emitted = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
        ids.push(observation_id(&emitted)?);
        effects.add(&emitted);
    }
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let open = apply_core_work(
        &core,
        effects.apply(
            Metric::new("freshness", "profile_open", label, elapsed, samples)
                .cpu(cpu)
                .count("observations", ids.len() as u64)
                .note(
                    "headless kind:0 opens; relay admission remains pending and no socket exists",
                ),
        ),
    );

    let order: Vec<_> = (0..ids.len()).collect();
    let close = close_phase(&mut core, &ids, &order, label);
    anyhow::ensure!(
        close.counts.get("wire_ops").copied().unwrap_or_default() == 0
            && close
                .counts
                .get("active_observations")
                .copied()
                .unwrap_or_default()
                == 0,
        "freshness stress teardown emitted wire work or retained observations"
    );
    Ok(vec![open, close])
}

#[cfg(test)]
#[path = "freshness/tests.rs"]
mod tests;
