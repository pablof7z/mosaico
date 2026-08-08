use std::time::Instant;

use anyhow::{Context, Result};
use nmp::mechanism::core::{Effect, EngineCore, EngineMsg};
use nmp_store::RedbStore;

use crate::args::Topology;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run_core(
    workload: &Workload,
    fixture: &DisposableStore,
    topology: Topology,
) -> Result<Vec<Metric>> {
    let query_count = workload.retained_live_queries(topology)?.len();
    let tick_store = RedbStore::open(fixture.path()).context("opening tick control store")?;
    let mut tick_core = EngineCore::new(tick_store, 8);
    tick_core.bench_reset_query_work();
    tick_core.bench_reset_lifecycle_work();
    tick_core.bench_reset_coverage_reads();
    let tick_started = Instant::now();
    let tick_cpu_started = process_cpu_time();
    let mut tick_samples = Samples::default();
    let mut tick_counts = EffectCounts::default();
    let now = nostr::Timestamp::now();
    for _ in 0..query_count {
        let effects = tick_samples.record(|| tick_core.handle(EngineMsg::Tick(now)));
        tick_counts.add(&effects);
    }
    let (tick_elapsed, tick_cpu) = elapsed_since(tick_started, tick_cpu_started);
    let (tick_index_rows, tick_event_values, tick_examined_rows) = tick_core.bench_query_work();
    let (tick_projection_reads, tick_router_compiles, tick_history_reads) =
        tick_core.bench_lifecycle_work();
    let tick_coverage_reads = tick_core.bench_coverage_reads();
    let tick = tick_counts.apply(
        Metric::new(
            "internal_control",
            "core_runtime_presubscribe_tick",
            topology.label(),
            tick_elapsed,
            tick_samples,
        )
        .cpu(tick_cpu)
        .count("projection_reads", tick_projection_reads)
        .count("router_compiles", tick_router_compiles)
        .count("history_projection_reads", tick_history_reads)
        .count("index_rows", tick_index_rows)
        .count("event_values", tick_event_values)
        .count("examined_rows", tick_examined_rows)
        .count("coverage_reads", tick_coverage_reads)
        .note("runtime executes this durable deadline/write sweep before every public subscribe"),
    );
    drop(tick_core);

    let store = RedbStore::open(fixture.path()).context("opening headless core store")?;
    let mut core = EngineCore::new(store, 8);
    let queries = workload.retained_live_queries(topology)?;
    core.bench_reset_query_work();
    core.bench_reset_lifecycle_work();
    core.bench_reset_coverage_reads();

    let started = Instant::now();
    let mut samples = Samples::default();
    let mut ids = Vec::with_capacity(queries.len());
    let mut counts = EffectCounts::default();
    for query in queries {
        let effects = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
        let id = effects
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Effect::EmitRows(id, _, _) => Some(*id),
                _ => None,
            })
            .context("headless core refused retained observation")?;
        ids.push(id);
        counts.add(&effects);
    }
    let open_elapsed = started.elapsed();
    let (index_rows, event_values, examined_rows) = core.bench_query_work();
    let (projection_reads, router_compiles, history_projection_reads) = core.bench_lifecycle_work();
    let coverage_reads = core.bench_coverage_reads();
    let open = counts.apply(
        Metric::new(
            "internal_control",
            "core_live_incremental_open",
            topology.label(),
            open_elapsed,
            samples,
        )
        .count("observations", ids.len() as u64)
        .count("projection_reads", projection_reads)
        .count("router_compiles", router_compiles)
        .count("history_projection_reads", history_projection_reads)
        .count("index_rows", index_rows)
        .count("event_values", event_values)
        .count("examined_rows", examined_rows)
        .count("coverage_reads", coverage_reads)
        .note("immediate cache seed per observation; relay admission remains pending"),
    );

    core.bench_reset_query_work();
    core.bench_reset_lifecycle_work();
    core.bench_reset_coverage_reads();
    let started = Instant::now();
    let mut samples = Samples::default();
    let mut counts = EffectCounts::default();
    let effects = samples.record(|| core.handle(EngineMsg::FlushWireAdmission));
    counts.add(&effects);
    let admission_elapsed = started.elapsed();
    let (index_rows, event_values, examined_rows) = core.bench_query_work();
    let (projection_reads, router_compiles, history_projection_reads) = core.bench_lifecycle_work();
    let coverage_reads = core.bench_coverage_reads();
    let admission = counts.apply(
        Metric::new(
            "internal_control",
            "core_pending_admission",
            topology.label(),
            admission_elapsed,
            samples,
        )
        .count("pending_observations", ids.len() as u64)
        .count("projection_reads", projection_reads)
        .count("router_compiles", router_compiles)
        .count("history_projection_reads", history_projection_reads)
        .count("index_rows", index_rows)
        .count("event_values", event_values)
        .count("examined_rows", examined_rows)
        .count("coverage_reads", coverage_reads)
        .note("one completed admission cohort; running REQs are not reconsidered"),
    );

    core.bench_reset_query_work();
    core.bench_reset_lifecycle_work();
    core.bench_reset_coverage_reads();
    let started = Instant::now();
    let mut samples = Samples::default();
    let mut counts = EffectCounts::default();
    for id in ids {
        let effects = samples.record(|| core.handle(EngineMsg::Unsubscribe(id)));
        counts.add(&effects);
    }
    let close_elapsed = started.elapsed();
    let (index_rows, event_values, examined_rows) = core.bench_query_work();
    let (projection_reads, router_compiles, history_projection_reads) = core.bench_lifecycle_work();
    let coverage_reads = core.bench_coverage_reads();
    let close = counts.apply(
        Metric::new(
            "internal_control",
            "core_live_incremental_close",
            topology.label(),
            close_elapsed,
            samples,
        )
        .count("projection_reads", projection_reads)
        .count("router_compiles", router_compiles)
        .count("history_projection_reads", history_projection_reads)
        .count("index_rows", index_rows)
        .count("event_values", event_values)
        .count("examined_rows", examined_rows)
        .count("coverage_reads", coverage_reads)
        .note("immutable withdrawal; no surviving-observer row reprojection"),
    );
    Ok(vec![tick, open, admission, close])
}

#[derive(Default)]
struct EffectCounts {
    effects: u64,
    wire_ops: u64,
    row_frames: u64,
    row_deltas: u64,
    evidence_frames: u64,
    diagnostics: u64,
}

impl EffectCounts {
    fn add(&mut self, effects: &[Effect]) {
        self.effects += effects.len() as u64;
        for effect in effects {
            match effect {
                Effect::Wire(delta) => {
                    self.wire_ops += delta
                        .ops
                        .iter()
                        .map(|(_, operations)| operations.len() as u64)
                        .sum::<u64>();
                }
                Effect::EmitRows(_, deltas, _) => {
                    self.row_frames += 1;
                    self.row_deltas += deltas.len() as u64;
                }
                Effect::EmitObservationEvidence(_, _) => self.evidence_frames += 1,
                Effect::EmitDiagnostics(_) => self.diagnostics += 1,
                _ => {}
            }
        }
    }

    fn apply(&self, metric: Metric) -> Metric {
        metric
            .count("effects", self.effects)
            .count("wire_ops", self.wire_ops)
            .count("row_frames", self.row_frames)
            .count("row_deltas", self.row_deltas)
            .count("evidence_frames", self.evidence_frames)
            .count("diagnostics", self.diagnostics)
    }
}
