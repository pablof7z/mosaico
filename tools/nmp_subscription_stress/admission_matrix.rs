use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{Effect, EngineCore, EngineMsg};
use nmp::{Binding, LiveQuery};
use nmp_store::RedbStore;

use crate::args::{Args, DemandShape, LifecycleSchedule};
use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::lifecycle::{
    close_phase, ensure_unique, flush_phase, flush_phase_capture, observation_id,
};
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run_active_attach(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
) -> Result<Vec<Metric>> {
    let store = RedbStore::open(fixture.path()).context("opening active-attach store")?;
    let mut core = EngineCore::new(store, 8);
    let mut queries = workload.matrix_queries(args.retained, DemandShape::ExactDuplicate)?;
    let first = queries.remove(0);
    let first_effects = core.handle(EngineMsg::Subscribe(first));
    let mut ids = vec![observation_id(&first_effects)?];
    let admitted = core.handle(EngineMsg::FlushWireAdmission(nostr::Timestamp::from(0u64)));
    let mut setup_counts = EffectCounts::default();
    setup_counts.add(&first_effects);
    setup_counts.add(&admitted);
    ensure!(
        setup_counts.wire_reqs == 1 && setup_counts.wire_closes == 0,
        "active-attach fixture must establish one immutable request"
    );

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut counts = EffectCounts::default();
    for query in queries {
        let effects = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
        ids.push(observation_id(&effects)?);
        counts.add(&effects);
    }
    ensure_unique(&ids)?;
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let label = format!("n={}:exact_duplicate:active_attach", args.retained);
    let attached = apply_core_work(
        &core,
        counts.apply(
            Metric::new(
                "matrix",
                "active_attach_open",
                label.clone(),
                elapsed,
                samples,
            )
            .cpu(cpu)
            .count("observations", ids.len() as u64)
            .count("unique_observation_ids", ids.len() as u64),
        ),
    );
    ensure!(
        attached.counts["router_compiles"] == 0
            && attached.counts["wire_reqs"] == 0
            && attached.counts["wire_closes"] == 0,
        "exact active coverage must attach without router or wire work"
    );
    let flush = flush_phase(&mut core, &label, "flush_after_active_attach");
    ensure!(
        flush.counts["effects"] == 0 && flush.counts["router_compiles"] == 0,
        "exact active attachment must leave no pending admission work"
    );
    let order: Vec<_> = (0..ids.len()).collect();
    let close = close_phase(&mut core, &ids, &order, &label);
    ensure!(
        close.counts["exact_atoms_closed"] == 1
            && close.counts["request_edges"] == 1
            && close.counts["requests_closed"] == 1
            && close.counts["wire_closes"] == 1
            && close.counts["pending_atoms_rebuilt"] == 0,
        "exact duplicate owners must retire their one physical request only at final close"
    );
    Ok(vec![attached, flush, close])
}

pub(crate) fn run_two_wave(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
    shape: DemandShape,
) -> Result<Vec<Metric>> {
    let store = RedbStore::open(fixture.path()).context("opening two-wave store")?;
    let mut core = EngineCore::new(store, 8);
    ensure!(matches!(
        shape,
        DemandShape::CompatibleDistinct | DemandShape::ProfileAuthors
    ));
    let queries = workload.matrix_queries(args.retained, shape)?;
    let split = queries.len().div_ceil(2);
    let (first, second) = queries.split_at(split);
    let mut ids = Vec::with_capacity(queries.len());
    for query in first {
        ids.push(observation_id(
            &core.handle(EngineMsg::Subscribe(query.clone())),
        )?);
    }
    let first_admission = core.handle(EngineMsg::FlushWireAdmission(nostr::Timestamp::from(0u64)));
    let first_wire_ids = wire_request_ids(&first_admission);
    let mut first_counts = EffectCounts::default();
    first_counts.add(&first_admission);
    ensure!(first_counts.wire_reqs > 0 && first_counts.wire_closes == 0);

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut open_counts = EffectCounts::default();
    for query in second {
        let effects = samples.record(|| core.handle(EngineMsg::Subscribe(query.clone())));
        ids.push(observation_id(&effects)?);
        open_counts.add(&effects);
    }
    ensure_unique(&ids)?;
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let label = format!("n={}:{}:two_wave", args.retained, shape.label());
    let second_open = apply_core_work(
        &core,
        open_counts.apply(
            Metric::new(
                "matrix",
                "second_wave_open",
                label.clone(),
                elapsed,
                samples,
            )
            .cpu(cpu)
            .count("observations", ids.len() as u64),
        ),
    );
    let (second_admission, second_effects) =
        flush_phase_capture(&mut core, &label, "second_wave_admission");
    ensure!(
        second_admission.counts["wire_reqs"] > 0 && second_admission.counts["wire_closes"] == 0,
        "a later uncovered cohort must add requests without rewriting sent requests"
    );
    let second_wire_ids = wire_request_ids(&second_effects);
    ensure!(
        first_wire_ids.is_disjoint(&second_wire_ids),
        "the second wave reused an already-sent request id instead of appending immutable work"
    );
    let expected_second = query_values(second, shape)?;
    ensure!(
        wire_values(&second_effects, shape) == expected_second,
        "the second admission wave did not cover its exact values once"
    );
    let order = crate::schedule::close_order(ids.len(), LifecycleSchedule::Reverse, args.seed);
    let close = close_phase(&mut core, &ids, &order, &label);
    ensure!(
        close.counts["request_edges"] == args.retained as u64
            && close.counts["coverage_edges_released"] == args.retained as u64
            && close.counts["wire_closes"] == (first_wire_ids.len() + second_wire_ids.len()) as u64
            && close.counts["projection_reads"] == 0
            && close.counts["router_compiles"] == 0
            && close.counts["pending_atoms_rebuilt"] == 0,
        "two-wave teardown was not exact delta withdrawal"
    );
    Ok(vec![second_open, second_admission, close])
}

pub(crate) fn wire_request_ids(effects: &[Effect]) -> BTreeSet<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Wire(delta) => Some(delta),
            _ => None,
        })
        .flat_map(|delta| &delta.ops)
        .flat_map(|(session, operations)| {
            operations
                .iter()
                .filter_map(move |operation| match operation {
                    nmp_router::WireOp::Req(id, _) => Some(format!("{session:?}:{id:?}")),
                    nmp_router::WireOp::Close(_) => None,
                })
        })
        .collect()
}

pub(crate) fn query_values(queries: &[LiveQuery], shape: DemandShape) -> Result<BTreeSet<String>> {
    let mut values = BTreeSet::new();
    for query in queries {
        let filter = &query.branches()[0].selection;
        match shape {
            DemandShape::CompatibleDistinct => {
                values.extend(filter.tags.values().flat_map(|binding| match binding {
                    Binding::Literal(values) => values.iter().cloned().collect::<Vec<_>>(),
                    _ => Vec::new(),
                }));
            }
            DemandShape::ProfileAuthors => {
                if let Some(Binding::Literal(authors)) = &filter.authors {
                    values.extend(authors.iter().cloned());
                }
            }
            _ => anyhow::bail!("wire value extraction does not support {shape:?}"),
        }
    }
    ensure!(values.len() == queries.len());
    Ok(values)
}

pub(crate) fn wire_values(effects: &[Effect], shape: DemandShape) -> BTreeSet<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Wire(delta) => Some(delta),
            _ => None,
        })
        .flat_map(|delta| &delta.ops)
        .flat_map(|(_, operations)| operations)
        .filter_map(|operation| match operation {
            nmp_router::WireOp::Req(_, filter) => Some(filter),
            nmp_router::WireOp::Close(_) => None,
        })
        .flat_map(|filter| match shape {
            DemandShape::CompatibleDistinct => filter
                .tags
                .values()
                .flat_map(|values| values.iter().cloned())
                .collect::<Vec<_>>(),
            DemandShape::ProfileAuthors => filter
                .authors
                .iter()
                .flat_map(|values| values.iter().cloned())
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}
