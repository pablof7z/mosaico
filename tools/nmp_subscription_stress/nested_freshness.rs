use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg};
use nmp::Freshness;
use nmp_store::{CoverageInterval, EventStore, RedbStore};
use nostr::Timestamp;

use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::execution::{
    accept_requests, concrete_revisions, relay_request_witnesses, wire_requests,
};
use crate::lifecycle::{close_phase_capture, flush_phase_capture, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

const NOW: u64 = 1_800_000_000;

pub(crate) fn run(workload: &Workload, fixture: &DisposableStore) -> Result<Vec<Metric>> {
    let mut metrics = run_case(
        workload,
        fixture,
        0,
        Freshness::CacheOnly,
        "live_inner_cache_only_outer",
        false,
    )?;
    metrics.extend(run_case(
        workload,
        fixture,
        1,
        Freshness::MaxAge { seconds: 3_600 },
        "live_inner_satisfied_max_age_outer",
        true,
    )?);
    Ok(metrics)
}

fn run_case(
    workload: &Workload,
    fixture: &DisposableStore,
    index: usize,
    outer_freshness: Freshness,
    label: &'static str,
    seed_current_coverage: bool,
) -> Result<Vec<Metric>> {
    let mut store = RedbStore::open(fixture.path()).context("opening nested freshness store")?;
    if seed_current_coverage {
        store.record_coverage(&[(
            workload.profile_atom(index),
            workload.relay().clone(),
            CoverageInterval::new(Timestamp::from(0), Timestamp::from(NOW - 60)),
        )])?;
    }
    let mut core = EngineCore::new(store, 8);
    core.handle(EngineMsg::Tick(Timestamp::from(NOW)));
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let effects = samples.record(|| {
        core.handle(EngineMsg::Subscribe(
            workload
                .nested_same_demand_query(index, outer_freshness)
                .expect("deterministic nested query"),
        ))
    });
    let id = observation_id(&effects)?;
    let revisions = concrete_revisions(&effects);
    let mut counts = EffectCounts::default();
    counts.add(&effects);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let open = apply_core_work(
        &core,
        counts.apply(
            Metric::new("matrix", "nested_freshness_open", label, elapsed, samples)
                .cpu(cpu)
                .count("observations", 1)
                .count("unique_observation_ids", 1),
        ),
    );
    ensure!(
        open.counts["request_target_handles"] == 1
            && open.counts["request_target_demand_keys"] == 1
            && open.counts["request_target_edges"] == 1
            && open.counts["request_target_refs"] == 1,
        "non-Live nested occurrence entered request execution ownership"
    );

    let (admission, admitted) = flush_phase_capture(&mut core, label, "nested_freshness_admission");
    let requests = wire_requests(&admitted);
    ensure!(
        requests.len() == 1
            && admission.counts["request_target_demand_keys_touched"] == 0
            && admission.counts["request_target_candidates_examined"] == 0,
        "planning must keep one request and defer execution projection until acceptance"
    );

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let accepted = samples.record(|| accept_requests(&mut core, &requests, 4));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let witnesses = relay_request_witnesses(&accepted);
    ensure!(
        witnesses.len() == 1 && witnesses[0].observation == id && !witnesses[0].replay,
        "nested non-Live occurrence received RelayRequest evidence"
    );
    let witness = &witnesses[0];
    let same_filter_occurrences = revisions
        .iter()
        .filter(|(_, _, _, hash)| *hash == witness.filter.hash())
        .collect::<Vec<_>>();
    ensure!(
        same_filter_occurrences.len() >= 2
            && same_filter_occurrences
                .iter()
                .map(|(_, path, _, _)| path)
                .collect::<BTreeSet<_>>()
                .len()
                >= 2
            && same_filter_occurrences
                .iter()
                .filter(|(observation, path, revision, _)| {
                    *observation == witness.observation
                        && path == &witness.path
                        && *revision == witness.filter_revision
                })
                .count()
                == 1,
        "RelayRequest evidence did not retain the exact Live occurrence path/revision"
    );
    let mut accepted_counts = EffectCounts::default();
    accepted_counts.add(&accepted);
    let handoff = apply_core_work(
        &core,
        accepted_counts.apply(
            Metric::new(
                "matrix",
                "nested_freshness_handoff",
                label,
                elapsed,
                samples,
            )
            .cpu(cpu)
            .count("relay_request_facts", 1)
            .count(
                "same_filter_occurrences",
                same_filter_occurrences.len() as u64,
            )
            .count("execution_occurrences", 1),
        ),
    );
    ensure!(
        handoff.counts["request_target_demand_keys_touched"] == 1
            && handoff.counts["request_target_candidates_examined"] == 1,
        "accepted same-DemandKey nested occurrences did not collapse to one Live execution target"
    );

    let (mut close, _) = close_phase_capture(&mut core, &[id], &[0], label);
    close.phase = "nested_freshness_close";
    ensure!(close.counts["wire_closes"] == 1);
    ensure_zero_census(&close)?;
    Ok(vec![open, admission, handoff, close])
}

#[cfg(test)]
#[path = "nested_freshness/tests.rs"]
mod tests;
