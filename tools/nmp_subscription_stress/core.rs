use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::{Context, Result};
use nmp::mechanism::core::{Effect, EngineCore, EngineMsg};
use nmp_store::RedbStore;
use nostr::JsonUtil;

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
            "core_counterfactual_tick_sweep",
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
        .note("counterfactual control for the pre-#1344 runtime path; candidate opens must not execute this sweep"),
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
    let effects =
        samples.record(|| core.handle(EngineMsg::FlushWireAdmission(nostr::Timestamp::from(0u64))));
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
pub(crate) struct EffectCounts {
    effects: u64,
    wire_ops: u64,
    pub(crate) wire_reqs: u64,
    pub(crate) wire_closes: u64,
    row_frames: u64,
    row_deltas: u64,
    evidence_frames: u64,
    diagnostics: u64,
    wire_author_occurrences: u64,
    wire_tag_occurrences: u64,
    wire_author_values: BTreeSet<String>,
    wire_tag_values: BTreeSet<String>,
    wire_reqs_with_limit: u64,
    wire_filter_bytes: u64,
    max_wire_filter_bytes: u64,
    max_authors_per_req: u64,
    max_tag_values_per_req: u64,
}

impl EffectCounts {
    pub(crate) fn add(&mut self, effects: &[Effect]) {
        self.effects += effects.len() as u64;
        for effect in effects {
            match effect {
                Effect::Wire(delta) => {
                    for operation in delta.ops.iter().flat_map(|(_, operations)| operations) {
                        self.wire_ops += 1;
                        match operation {
                            nmp_router::WireOp::Req(_, filter) => {
                                self.wire_reqs += 1;
                                let authors = filter.authors.as_ref().map_or(0, BTreeSet::len);
                                let tags = filter.tags.values().map(BTreeSet::len).sum::<usize>();
                                self.max_authors_per_req =
                                    self.max_authors_per_req.max(authors as u64);
                                self.max_tag_values_per_req =
                                    self.max_tag_values_per_req.max(tags as u64);
                                if let Some(values) = &filter.authors {
                                    self.wire_author_occurrences += values.len() as u64;
                                    self.wire_author_values.extend(values.iter().cloned());
                                }
                                for values in filter.tags.values() {
                                    self.wire_tag_occurrences += values.len() as u64;
                                    self.wire_tag_values.extend(values.iter().cloned());
                                }
                                self.wire_reqs_with_limit += u64::from(filter.limit.is_some());
                                let bytes = filter.to_nostr().as_json().len() as u64;
                                self.wire_filter_bytes += bytes;
                                self.max_wire_filter_bytes = self.max_wire_filter_bytes.max(bytes);
                            }
                            nmp_router::WireOp::Close(_) => self.wire_closes += 1,
                        }
                    }
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

    pub(crate) fn apply(&self, metric: Metric) -> Metric {
        metric
            .count("effects", self.effects)
            .count("wire_ops", self.wire_ops)
            .count("wire_reqs", self.wire_reqs)
            .count("wire_closes", self.wire_closes)
            .count("row_frames", self.row_frames)
            .count("row_deltas", self.row_deltas)
            .count("evidence_frames", self.evidence_frames)
            .count("diagnostics", self.diagnostics)
            .count("wire_author_occurrences", self.wire_author_occurrences)
            .count("wire_unique_authors", self.wire_author_values.len() as u64)
            .count("wire_tag_occurrences", self.wire_tag_occurrences)
            .count("wire_unique_tag_values", self.wire_tag_values.len() as u64)
            .count("wire_reqs_with_limit", self.wire_reqs_with_limit)
            .count("wire_filter_bytes", self.wire_filter_bytes)
            .count("max_wire_filter_bytes", self.max_wire_filter_bytes)
            .count("max_authors_per_req", self.max_authors_per_req)
            .count("max_tag_values_per_req", self.max_tag_values_per_req)
    }
}
