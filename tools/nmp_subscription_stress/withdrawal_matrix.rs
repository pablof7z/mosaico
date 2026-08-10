use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg};
use nmp_store::RedbStore;

use crate::admission_matrix::{query_values, wire_values};
use crate::args::{Args, DemandShape};
use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::lifecycle::{close_phase, flush_phase, flush_phase_capture, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run_duplicate_withdrawal(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
) -> Result<Vec<Metric>> {
    ensure!(args.retained > 1);
    let store = RedbStore::open(fixture.path()).context("opening duplicate withdrawal store")?;
    let mut core = EngineCore::new(store, 8);
    let queries = workload.matrix_queries(args.retained, DemandShape::ExactDuplicate)?;
    let mut ids = Vec::with_capacity(queries.len());
    for query in queries {
        ids.push(observation_id(&core.handle(EngineMsg::Subscribe(query)))?);
    }
    let admitted = core.handle(EngineMsg::FlushWireAdmission(nostr::Timestamp::from(0u64)));
    let mut admission_counts = EffectCounts::default();
    admission_counts.add(&admitted);
    ensure!(admission_counts.wire_reqs == 1 && admission_counts.wire_closes == 0);

    let label = format!("n={}:exact_duplicate:per_withdrawal", args.retained);
    let nonfinal_ids = &ids[..ids.len() - 1];
    let nonfinal_order: Vec<_> = (0..nonfinal_ids.len()).collect();
    let nonfinal = close_phase(&mut core, nonfinal_ids, &nonfinal_order, &label);
    ensure!(
        value(&nonfinal, "handles_detached") == nonfinal_ids.len() as u64
            && value(&nonfinal, "resolver_delta_ops") == 0
            && value(&nonfinal, "exact_atoms_closed") == 0
            && value(&nonfinal, "request_edges") == 0
            && value(&nonfinal, "coverage_edges_released") == 0
            && value(&nonfinal, "diagnostic_refreshes") == 0
            && value(&nonfinal, "diagnostic_snapshots_built") == 0
            && value(&nonfinal, "evidence_candidates_examined") == 0
            && value(&nonfinal, "projection_reads") == 0
            && value(&nonfinal, "router_compiles") == 0
            && value(&nonfinal, "wire_ops") == 0
            && value(&nonfinal, "pending_atoms_rebuilt") == 0
            && value(&nonfinal, "active_physical_requests") == 1,
        "a non-final duplicate owner performed shared wire or projection work"
    );

    let final_owner = close_phase(&mut core, &ids[ids.len() - 1..], &[0], &label);
    ensure!(
        value(&final_owner, "handles_detached") == 1
            && value(&final_owner, "resolver_delta_ops") == 1
            && value(&final_owner, "exact_atoms_closed") == 1
            && value(&final_owner, "request_edges") == 1
            && value(&final_owner, "requests_closed") == 1
            && value(&final_owner, "coverage_edges_released") == 1
            && value(&final_owner, "wire_closes") == 1
            && value(&final_owner, "pending_atoms_rebuilt") == 0,
        "the final duplicate owner did not retire exactly one physical request"
    );
    ensure_zero_census(&final_owner)?;
    Ok(vec![nonfinal, final_owner])
}

pub(crate) fn run_detached_reattach(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
) -> Result<Vec<Metric>> {
    ensure!(args.retained > 1);
    let store = RedbStore::open(fixture.path()).context("opening detached reattach store")?;
    let mut core = EngineCore::new(store, 8);
    let queries = workload.matrix_queries(2, DemandShape::CompatibleDistinct)?;
    let first_query = queries[0].clone();
    let first = observation_id(&core.handle(EngineMsg::Subscribe(first_query.clone())))?;
    let retained = observation_id(&core.handle(EngineMsg::Subscribe(queries[1].clone())))?;
    let (admission, _) = flush_phase_capture(&mut core, "reattach_setup", "reattach_setup");
    ensure!(
        value(&admission, "wire_reqs") == 1 && value(&admission, "wire_closes") == 0,
        "reattach fixture must start with one shared request"
    );

    let label = format!("n={}:compatible_distinct:detach_reattach", args.retained);
    let detached = close_phase(&mut core, &[first], &[0], &label);
    ensure_covered_atom_close(&detached).context("closing A before reattach")?;

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let reopened_effects = samples.record(|| core.handle(EngineMsg::Subscribe(first_query)));
    let reopened = observation_id(&reopened_effects)?;
    let mut counts = EffectCounts::default();
    counts.add(&reopened_effects);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let reattach = apply_core_work(
        &core,
        counts.apply(
            Metric::new(
                "matrix",
                "detached_demand_reattach",
                &label,
                elapsed,
                samples,
            )
            .cpu(cpu),
        ),
    );
    ensure!(
        value(&reattach, "row_frames") == 1
            && value(&reattach, "evidence_frames") == 1
            && value(&reattach, "projection_reads") == 1
            && value(&reattach, "router_compiles") == 0
            && value(&reattach, "wire_ops") == 0
            && value(&reattach, "pending_wire_atoms") == 0
            && value(&reattach, "active_physical_requests") == 1,
        "reattaching exact demand did not reuse retained physical coverage"
    );
    let flush = flush_phase(&mut core, &label, "flush_after_detached_reattach");
    ensure!(value(&flush, "effects") == 0 && value(&flush, "router_compiles") == 0);

    let sibling_close = close_phase(&mut core, &[retained], &[0], &label);
    ensure_covered_atom_close(&sibling_close).context("closing B after A reattached")?;
    let final_owner = close_phase(&mut core, &[reopened], &[0], &label);
    ensure!(
        value(&final_owner, "exact_atoms_closed") == 1
            && value(&final_owner, "request_edges") == 1
            && value(&final_owner, "requests_closed") == 1
            && value(&final_owner, "coverage_edges_released") == 2
            && value(&final_owner, "wire_closes") == 1,
        "final retained owner did not retire the reattached request exactly once"
    );
    ensure_zero_census(&final_owner)?;
    Ok(vec![detached, reattach, flush, sibling_close, final_owner])
}

pub(crate) fn run_partial_pending_cancellation(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
    shape: DemandShape,
) -> Result<Vec<Metric>> {
    ensure!(matches!(
        shape,
        DemandShape::CompatibleDistinct | DemandShape::ProfileAuthors
    ));
    let mut cancel_counts = vec![
        args.retained.div_ceil(100),
        args.retained / 2,
        args.retained - 1,
    ];
    cancel_counts.sort_unstable();
    cancel_counts.dedup();
    let mut metrics = Vec::new();
    for cancelled in cancel_counts {
        metrics.extend(run_partial_case(args, workload, fixture, shape, cancelled)?);
    }
    Ok(metrics)
}

fn run_partial_case(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
    shape: DemandShape,
    cancelled: usize,
) -> Result<Vec<Metric>> {
    ensure!(cancelled > 0 && cancelled < args.retained);
    let store = RedbStore::open(fixture.path()).context("opening partial cancellation store")?;
    let mut core = EngineCore::new(store, 8);
    let queries = workload.matrix_queries(args.retained, shape)?;
    let mut ids = Vec::with_capacity(queries.len());
    for query in &queries {
        ids.push(observation_id(
            &core.handle(EngineMsg::Subscribe(query.clone())),
        )?);
    }
    let label = format!(
        "n={}:{}:partial_pending_cancel={cancelled}",
        args.retained,
        shape.label()
    );
    let cancelled_order: Vec<_> = (0..cancelled).collect();
    let cancelled_metric = close_phase(&mut core, &ids[..cancelled], &cancelled_order, &label);
    ensure!(
        value(&cancelled_metric, "handles_detached") == cancelled as u64
            && value(&cancelled_metric, "resolver_delta_ops") == cancelled as u64
            && value(&cancelled_metric, "exact_atoms_closed") == cancelled as u64
            && value(&cancelled_metric, "pending_atoms_rebuilt") == 0
            && value(&cancelled_metric, "request_edges") == 0
            && value(&cancelled_metric, "projection_reads") == 0
            && value(&cancelled_metric, "coverage_reads") == 0
            && value(&cancelled_metric, "evidence_candidates_examined") == 0
            && value(&cancelled_metric, "diagnostic_snapshots_built") == 0
            && value(&cancelled_metric, "router_compiles") == 0
            && value(&cancelled_metric, "wire_ops") == 0,
        "partial pending cancellation performed broad or physical work"
    );

    let (admission, effects) = flush_phase_capture(&mut core, &label, "survivor_admission");
    let expected = query_values(&queries[cancelled..], shape)?;
    ensure!(
        value(&admission, "router_compiles") == 1
            && wire_values(&effects, shape) == expected
            && value(&admission, "wire_reqs_with_limit") == 0,
        "admission did not contain exactly the surviving unlimited demand"
    );

    let survivor_ids = &ids[cancelled..];
    let survivor_order: Vec<_> = (0..survivor_ids.len()).collect();
    let final_close = close_phase(&mut core, survivor_ids, &survivor_order, &label);
    ensure!(
        value(&final_close, "handles_detached") == survivor_ids.len() as u64
            && value(&final_close, "exact_atoms_closed") == survivor_ids.len() as u64
            && value(&final_close, "request_edges") == survivor_ids.len() as u64
            && value(&final_close, "coverage_edges_released") == survivor_ids.len() as u64
            && value(&final_close, "wire_closes") == value(&admission, "wire_reqs")
            && value(&final_close, "pending_atoms_rebuilt") == 0
            && value(&final_close, "projection_reads") == 0
            && value(&final_close, "coverage_reads") == 0
            && value(&final_close, "evidence_candidates_examined") == 0
            && value(&final_close, "diagnostic_snapshots_built") == 0
            && value(&final_close, "router_compiles") == 0,
        "survivor teardown was not exact delta withdrawal"
    );
    ensure_zero_census(&final_close)?;
    Ok(vec![cancelled_metric, admission, final_close])
}

fn ensure_covered_atom_close(metric: &Metric) -> Result<()> {
    ensure!(
        value(metric, "handles_detached") == 1
            && value(metric, "resolver_delta_ops") == 1
            && value(metric, "exact_atoms_closed") == 1
            && value(metric, "request_edges") == 1
            && value(metric, "requests_closed") == 0
            && value(metric, "coverage_edges_released") == 0
            && value(metric, "wire_ops") == 0
            && value(metric, "projection_reads") == 0
            && value(metric, "router_compiles") == 0
            && value(metric, "pending_atoms_rebuilt") == 0
            && value(metric, "active_physical_requests") == 1,
        "closing one covered atom retired shared physical coverage"
    );
    Ok(())
}

fn value(metric: &Metric, key: &str) -> u64 {
    metric.counts.get(key).copied().unwrap_or_default()
}

#[cfg(test)]
#[path = "withdrawal_matrix/tests.rs"]
mod tests;
