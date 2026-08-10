use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg};
use nmp::{AccessContext, SourceAuthority};
use nmp_grammar::ContextualAtom;
use nmp_router::DemandKey;
use nmp_store::{coverage_key, RedbStore};

use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::execution::{accept_requests, relay_request_witnesses, wire_requests};
use crate::lifecycle::{close_phase_capture, ensure_unique, flush_phase_capture, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run(workload: &Workload, fixture: &DisposableStore) -> Result<Vec<Metric>> {
    let store = RedbStore::open(fixture.path()).context("opening DemandKey matrix store")?;
    let mut core = EngineCore::new(store, 8);
    let queries = workload.demand_key_distinct_queries()?;
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut counts = EffectCounts::default();
    let mut ids = Vec::new();
    for query in queries {
        let effects = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
        ids.push(observation_id(&effects)?);
        counts.add(&effects);
    }
    ensure_unique(&ids)?;
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let open = apply_core_work(
        &core,
        counts.apply(
            Metric::new(
                "matrix",
                "demand_key_open",
                "same_coverage",
                elapsed,
                samples,
            )
            .cpu(cpu)
            .count("observations", ids.len() as u64)
            .count("unique_observation_ids", ids.len() as u64),
        ),
    );

    let (admission, admitted) = flush_phase_capture(
        &mut core,
        "same_coverage:window_distinct",
        "demand_key_admission",
    );
    let requests = wire_requests(&admitted);
    ensure!(
        requests.len() == 2
            && requests
                .iter()
                .map(|request| request.sub_id.clone())
                .collect::<BTreeSet<_>>()
                .len()
                == 2,
        "window-distinct demand must emit two REQs"
    );
    let atoms = requests
        .iter()
        .map(|request| ContextualAtom {
            filter: request.filter.clone(),
            source: SourceAuthority::Pinned(BTreeSet::from([workload.relay().clone()])),
            access: AccessContext::Public,
            routing_evidence: BTreeSet::new(),
        })
        .collect::<Vec<_>>();
    ensure!(
        atoms
            .iter()
            .map(coverage_key)
            .collect::<BTreeSet<_>>()
            .len()
            == 1,
        "window-erased CoverageKey must be identical"
    );
    ensure!(
        atoms
            .iter()
            .map(DemandKey::for_atom)
            .collect::<BTreeSet<_>>()
            .len()
            == 2,
        "since/until/limit must remain distinct DemandKeys"
    );
    ensure!(
        admission.counts["request_target_demand_keys_touched"] == 0
            && admission.counts["request_target_candidates_examined"] == 0
            && admission.counts["router_request_edges_appended"] == 2,
        "planning must not project request execution before local acceptance"
    );

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let accepted = samples.record(|| accept_requests(&mut core, &requests, 1));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let witnesses = relay_request_witnesses(&accepted);
    let by_observation = witnesses
        .iter()
        .fold(BTreeMap::new(), |mut counts, witness| {
            *counts.entry(witness.observation).or_insert(0u64) += 1;
            counts
        });
    ensure!(
        by_observation == BTreeMap::from([(ids[0], 1), (ids[1], 1)]),
        "RelayRequest evidence crossed between window-distinct observations"
    );
    for witness in &witnesses {
        let expected = if witness.filter.limit.is_some() {
            ids[1]
        } else {
            ids[0]
        };
        ensure!(witness.observation == expected && !witness.replay);
    }
    let mut accepted_counts = EffectCounts::default();
    accepted_counts.add(&accepted);
    let handoff = apply_core_work(
        &core,
        accepted_counts.apply(
            Metric::new(
                "matrix",
                "demand_key_handoff",
                "same_coverage",
                elapsed,
                samples,
            )
            .cpu(cpu)
            .count("relay_request_facts", witnesses.len() as u64),
        ),
    );
    ensure!(
        handoff.counts["request_target_demand_keys_touched"] == 2
            && handoff.counts["request_target_candidates_examined"] == 2,
        "each accepted immutable request must inspect only its exact app target"
    );

    let (mut first_close, _) =
        close_phase_capture(&mut core, &ids[..1], &[0], "same_coverage:unbounded_first");
    first_close.phase = "demand_key_unbounded_close";
    ensure!(
        first_close.counts["wire_closes"] == 1
            && first_close.counts["wire_reqs"] == 0
            && first_close.counts["active_physical_requests"] == 1,
        "closing unbounded demand rewrote or retired the limited request"
    );
    let (mut final_close, _) =
        close_phase_capture(&mut core, &ids[1..], &[0], "same_coverage:limited_final");
    final_close.phase = "demand_key_limited_close";
    ensure!(final_close.counts["wire_closes"] == 1);
    ensure_zero_census(&final_close)?;
    Ok(vec![open, admission, handoff, first_close, final_close])
}

#[cfg(test)]
#[path = "demand_key_matrix/tests.rs"]
mod tests;
