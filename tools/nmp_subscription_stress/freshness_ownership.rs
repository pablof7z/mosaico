use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg, ObservationId};
use nmp_store::RedbStore;

use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::execution::{accept_requests, relay_request_witnesses, wire_requests};
use crate::lifecycle::{close_phase_capture, ensure_unique, flush_phase_capture, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run(
    workload: &Workload,
    fixture: &DisposableStore,
    pairs: usize,
) -> Result<Vec<Metric>> {
    let mut metrics = Vec::new();
    metrics.extend(run_order(workload, fixture, pairs, true)?);
    metrics.extend(run_order(workload, fixture, pairs, false)?);
    Ok(metrics)
}

fn run_order(
    workload: &Workload,
    fixture: &DisposableStore,
    pairs: usize,
    live_first: bool,
) -> Result<Vec<Metric>> {
    ensure!(pairs > 0);
    let store = RedbStore::open(fixture.path()).context("opening freshness ownership store")?;
    let mut core = EngineCore::new(store, pairs.max(8));
    let queries = workload.live_cache_pairs(pairs)?;
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut counts = EffectCounts::default();
    let mut live = Vec::with_capacity(pairs);
    let mut cached = Vec::with_capacity(pairs);
    for (live_query, cached_query) in queries {
        let effects = samples.record(|| core.handle(EngineMsg::Subscribe(live_query)));
        live.push(observation_id(&effects)?);
        counts.add(&effects);
        let effects = samples.record(|| core.handle(EngineMsg::Subscribe(cached_query)));
        cached.push(observation_id(&effects)?);
        counts.add(&effects);
    }
    let mut all = live.clone();
    all.extend(&cached);
    ensure_unique(&all)?;
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let order = if live_first {
        "live_first"
    } else {
        "cache_only_first"
    };
    let label = format!("pairs={pairs}:{order}");
    let open = apply_core_work(
        &core,
        counts.apply(
            Metric::new("matrix", "live_cache_open", &label, elapsed, samples)
                .cpu(cpu)
                .count("observations", all.len() as u64)
                .count("live_observations", live.len() as u64)
                .count("cache_only_observations", cached.len() as u64)
                .count("unique_observation_ids", all.len() as u64),
        ),
    );
    ensure!(
        open.counts["request_target_handles"] == all.len() as u64
            && open.counts["request_target_demand_keys"] == live.len() as u64
            && open.counts["request_target_edges"] == live.len() as u64
            && open.counts["request_target_refs"] == live.len() as u64,
        "CacheOnly occurrences entered the active request-target reverse index"
    );

    let (admission, admitted) = flush_phase_capture(&mut core, &label, "live_cache_admission");
    let requests = wire_requests(&admitted);
    ensure!(!requests.is_empty());
    ensure!(
        admission.counts["request_target_demand_keys_touched"] == 0
            && admission.counts["request_target_candidates_examined"] == 0,
        "planning must not project request execution before local acceptance"
    );

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let accepted = samples.record(|| accept_requests(&mut core, &requests, 2));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let witnesses = relay_request_witnesses(&accepted);
    let witness_ids = witnesses
        .iter()
        .map(|witness| witness.observation)
        .collect::<BTreeSet<_>>();
    ensure!(
        witness_ids == live.iter().copied().collect()
            && witnesses.len() == live.len()
            && witnesses.iter().all(|witness| !witness.replay),
        "RelayRequest evidence was missing from Live or crossed into CacheOnly"
    );
    let mut accepted_counts = EffectCounts::default();
    accepted_counts.add(&accepted);
    let handoff = apply_core_work(
        &core,
        accepted_counts.apply(
            Metric::new("matrix", "live_cache_handoff", &label, elapsed, samples)
                .cpu(cpu)
                .count("relay_request_facts", witnesses.len() as u64),
        ),
    );
    ensure!(
        handoff.counts["request_target_demand_keys_touched"] == live.len() as u64
            && handoff.counts["request_target_candidates_examined"] == live.len() as u64,
        "accepted shared requests must visit every Live target and no CacheOnly sibling"
    );

    let (first_ids, last_ids) = if live_first {
        (&live, &cached)
    } else {
        (&cached, &live)
    };
    let mut first = close_group(&mut core, first_ids, &label);
    first.phase = if live_first {
        "live_close_with_cache_siblings"
    } else {
        "cache_only_close_with_live_siblings"
    };
    let expected_first_closes = if live_first { requests.len() as u64 } else { 0 };
    ensure!(first.counts["wire_closes"] == expected_first_closes);
    if live_first {
        ensure!(
            first.counts["active_physical_requests"] == 0
                && first.counts["active_observations"] == cached.len() as u64,
            "CacheOnly siblings retained Live wire ownership"
        );
    } else {
        ensure!(
            first.counts["active_physical_requests"] == requests.len() as u64
                && first.counts["active_observations"] == live.len() as u64,
            "closing CacheOnly siblings changed Live wire ownership"
        );
    }
    let mut final_close = close_group(&mut core, last_ids, &label);
    final_close.phase = if live_first {
        "cache_only_final_close"
    } else {
        "live_final_close"
    };
    let expected_final_closes = if live_first { 0 } else { requests.len() as u64 };
    ensure!(final_close.counts["wire_closes"] == expected_final_closes);
    ensure_zero_census(&final_close)?;
    Ok(vec![open, admission, handoff, first, final_close])
}

fn close_group(core: &mut EngineCore<RedbStore>, ids: &[ObservationId], label: &str) -> Metric {
    let order = (0..ids.len()).collect::<Vec<_>>();
    close_phase_capture(core, ids, &order, label).0
}

#[cfg(test)]
#[path = "freshness_ownership/tests.rs"]
mod tests;
