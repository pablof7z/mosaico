use std::collections::HashSet;
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{Effect, EngineCore, EngineMsg, ObservationId};
use nmp_store::RedbStore;

use crate::args::{Args, DemandShape, LifecycleSchedule};
use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::execution::{accept_requests, wire_requests};
use crate::matrix_oracle;
use crate::measure::{elapsed_since, process_cpu_time, resources, Metric, Samples};
use crate::schedule::close_order;
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
    shape: DemandShape,
    schedule: LifecycleSchedule,
) -> Result<Vec<Metric>> {
    let store = RedbStore::open(fixture.path()).context("opening lifecycle matrix store")?;
    let queries = workload.matrix_queries(args.retained, shape)?;
    let mut core = EngineCore::new(store, queries.len().max(8));
    let label = format!("n={}:{}:{}", args.retained, shape.label(), schedule.label());
    if schedule == LifecycleSchedule::Interleaved {
        return run_interleaved(&mut core, queries, label);
    }

    reset(&mut core);
    let before = resources();
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut effects = EffectCounts::default();
    let mut ids = Vec::with_capacity(queries.len());
    for query in queries {
        let emitted = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
        ids.push(observation_id(&emitted)?);
        effects.add(&emitted);
    }
    ensure_unique(&ids)?;
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let after = resources();
    let open = apply_core_work(
        &core,
        effects.apply(
            Metric::new(
                "matrix",
                "observation_open",
                label.clone(),
                elapsed,
                samples,
            )
            .cpu(cpu)
            .count("observations", ids.len() as u64)
            .count("unique_observation_ids", ids.len() as u64)
            .count("fds_before", before.open_fds)
            .count("rss_before_bytes", before.current_rss_bytes)
            .count("rss_after_bytes", after.current_rss_bytes)
            .count("physical_footprint_bytes", after.physical_footprint_bytes)
            .count("peak_rss_bytes", after.peak_rss_bytes),
        ),
    );

    let order = close_order(ids.len(), schedule, args.seed);
    if schedule == LifecycleSchedule::BeforeAdmission {
        let close = close_phase(&mut core, &ids, &order, &label);
        let admission = flush_phase(&mut core, &label, "flush_after_all_cancelled");
        matrix_oracle::validate_lifecycle(
            &open,
            &admission,
            None,
            &close,
            shape,
            schedule,
            ids.len(),
        )?;
        return Ok(vec![open, admission, close]);
    }

    let (admission, admitted) = flush_phase_capture(&mut core, &label, "pending_admission_flush");
    let requests = wire_requests(&admitted);
    let handoff = handoff_phase(&mut core, &requests, &label);
    let close = close_phase(&mut core, &ids, &order, &label);
    matrix_oracle::validate_lifecycle(
        &open,
        &admission,
        Some(&handoff),
        &close,
        shape,
        schedule,
        ids.len(),
    )?;
    Ok(vec![open, admission, handoff, close])
}

fn run_interleaved(
    core: &mut EngineCore<RedbStore>,
    queries: Vec<nmp::LiveQuery>,
    label: String,
) -> Result<Vec<Metric>> {
    reset(core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut effects = EffectCounts::default();
    let mut ids = Vec::with_capacity(queries.len());
    for query in queries {
        let emitted = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
        let id = observation_id(&emitted)?;
        effects.add(&emitted);
        effects.add(&samples.record(|| core.handle(EngineMsg::Unsubscribe(id))));
        ids.push(id);
    }
    ensure_unique(&ids)?;
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let churn = apply_core_work(
        core,
        effects.apply(
            Metric::new(
                "matrix",
                "interleaved_open_close",
                label.clone(),
                elapsed,
                samples,
            )
            .cpu(cpu)
            .count("observations", ids.len() as u64)
            .count("unique_observation_ids", ids.len() as u64),
        ),
    );
    let flush = flush_phase(core, &label, "flush_after_interleaved_cancel");
    matrix_oracle::validate_interleaved(&churn, &flush, ids.len())?;
    Ok(vec![churn, flush])
}

pub(crate) fn close_phase(
    core: &mut EngineCore<RedbStore>,
    ids: &[ObservationId],
    order: &[usize],
    label: &str,
) -> Metric {
    close_phase_capture(core, ids, order, label).0
}

pub(crate) fn close_phase_capture(
    core: &mut EngineCore<RedbStore>,
    ids: &[ObservationId],
    order: &[usize],
    label: &str,
) -> (Metric, Vec<Effect>) {
    reset(core);
    let before = resources();
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut effects = EffectCounts::default();
    let mut emitted = Vec::new();
    for index in order {
        let next = samples.record(|| core.handle(EngineMsg::Unsubscribe(ids[*index])));
        effects.add(&next);
        emitted.extend(next);
    }
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let after = resources();
    (
        apply_core_work(
            core,
            effects.apply(
                Metric::new("matrix", "observation_close", label, elapsed, samples)
                    .cpu(cpu)
                    .count("observations", ids.len() as u64)
                    .count("fds_before", before.open_fds)
                    .count("fds_after", after.open_fds)
                    .count("rss_before_bytes", before.current_rss_bytes)
                    .count("rss_after_bytes", after.current_rss_bytes)
                    .count("physical_footprint_bytes", after.physical_footprint_bytes)
                    .count("nmp_threads_live_after", after.nmp_threads_live)
                    .count("peak_rss_bytes", after.peak_rss_bytes),
            ),
        ),
        emitted,
    )
}

pub(crate) fn flush_phase(
    core: &mut EngineCore<RedbStore>,
    label: &str,
    phase: &'static str,
) -> Metric {
    flush_phase_capture(core, label, phase).0
}

fn handoff_phase(
    core: &mut EngineCore<RedbStore>,
    requests: &[crate::execution::WireRequest],
    label: &str,
) -> Metric {
    reset(core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut effects = EffectCounts::default();
    let emitted = samples.record(|| accept_requests(core, requests, 1));
    effects.add(&emitted);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    apply_core_work(
        core,
        effects.apply(Metric::new("matrix", "request_handoff", label, elapsed, samples).cpu(cpu)),
    )
}

pub(crate) fn flush_phase_capture(
    core: &mut EngineCore<RedbStore>,
    label: &str,
    phase: &'static str,
) -> (Metric, Vec<Effect>) {
    reset(core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut effects = EffectCounts::default();
    let emitted =
        samples.record(|| core.handle(EngineMsg::FlushWireAdmission(nostr::Timestamp::from(0u64))));
    effects.add(&emitted);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    (
        apply_core_work(
            core,
            effects.apply(Metric::new("matrix", phase, label, elapsed, samples).cpu(cpu)),
        ),
        emitted,
    )
}

pub(crate) fn observation_id(effects: &[Effect]) -> Result<ObservationId> {
    effects
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::EmitRows(id, _, _) => Some(*id),
            _ => None,
        })
        .context("matrix observation did not emit its opening frame")
}

pub(crate) fn ensure_unique(ids: &[ObservationId]) -> Result<()> {
    let unique: HashSet<_> = ids.iter().copied().collect();
    ensure!(
        unique.len() == ids.len(),
        "{} opens returned only {} unique observation ids",
        ids.len(),
        unique.len()
    );
    Ok(())
}

#[cfg(test)]
#[path = "lifecycle/tests.rs"]
mod tests;
