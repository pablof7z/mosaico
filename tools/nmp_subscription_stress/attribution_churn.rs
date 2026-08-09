use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg, ObservationId};
use nmp_store::RedbStore;

use crate::args::{Args, DemandShape};
use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::lifecycle::{close_phase, flush_phase, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

/// Keep one profile observation alive while disjoint profile cohorts churn.
/// Departed attribution shapes must disappear without waiting for total zero.
pub(crate) fn run(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
) -> Result<Vec<Metric>> {
    ensure!(args.retained > 2);
    let store = RedbStore::open(fixture.path()).context("opening attribution churn store")?;
    let mut core = EngineCore::new(store, 8);
    let queries = workload.matrix_queries(args.retained, DemandShape::ProfileAuthors)?;
    let standing = observation_id(&core.handle(EngineMsg::Subscribe(queries[0].clone())))?;
    let standing_admission = flush_phase(
        &mut core,
        &format!("n={}:profile_authors:standing", args.retained),
        "standing_admission",
    );
    ensure!(value(&standing_admission, "wire_reqs") == 1);

    let split = 1 + (queries.len() - 1) / 2;
    let mut metrics = vec![standing_admission];
    for (wave, cohort) in [&queries[1..split], &queries[split..]]
        .into_iter()
        .enumerate()
    {
        let label = format!("n={}:profile_authors:churn_wave={wave}", args.retained);
        let (open, ids) = open_wave(&mut core, cohort, &label)?;
        let admission = flush_phase(&mut core, &label, "churn_admission");
        ensure!(
            value(&admission, "router_compiles") == 1
                && value(&admission, "wire_reqs") == 1
                && value(&admission, "pending_atoms_rebuilt") == 0,
            "a churn cohort performed global pending reconstruction"
        );
        let order: Vec<_> = (0..ids.len()).collect();
        let close = close_phase(&mut core, &ids, &order, &label);
        ensure!(
            value(&close, "active_observations") == 1
                && value(&close, "active_physical_requests") == 1
                && value(&close, "attribution_shape_keys") == 1
                && value(&close, "pending_wire_atoms") == 0,
            "departed attribution state survived while the standing owner remained"
        );
        metrics.extend([open, admission, close]);
    }

    let final_close = close_phase(&mut core, &[standing], &[0], "profile_churn_final");
    ensure_zero_census(&final_close)?;
    metrics.push(final_close);
    Ok(metrics)
}

fn open_wave(
    core: &mut EngineCore<RedbStore>,
    queries: &[nmp::LiveQuery],
    label: &str,
) -> Result<(Metric, Vec<ObservationId>)> {
    reset(core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut effects = EffectCounts::default();
    let mut ids = Vec::with_capacity(queries.len());
    for query in queries {
        let emitted = samples.record(|| core.handle(EngineMsg::Subscribe(query.clone())));
        ids.push(observation_id(&emitted)?);
        effects.add(&emitted);
    }
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let metric = apply_core_work(
        core,
        effects.apply(
            Metric::new("matrix", "churn_open", label, elapsed, samples)
                .cpu(cpu)
                .count("observations", ids.len() as u64),
        ),
    );
    Ok((metric, ids))
}

fn value(metric: &Metric, key: &str) -> u64 {
    metric.counts.get(key).copied().unwrap_or_default()
}

#[cfg(test)]
#[path = "attribution_churn/tests.rs"]
mod tests;
