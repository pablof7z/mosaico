use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg};
use nmp_grammar::RelaySessionKey;
use nmp_store::RedbStore;
use nmp_transport::RelayHandle;

use crate::args::DemandShape;
use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::execution::{
    accept_requests, concrete_revisions, eose_request, request_settled_witnesses, wire_requests,
};
use crate::lifecycle::{close_phase_capture, flush_phase_capture, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run(workload: &Workload, fixture: &DisposableStore) -> Result<Vec<Metric>> {
    let mut metrics = run_order(workload, fixture, true)?;
    metrics.extend(run_order(workload, fixture, false)?);
    Ok(metrics)
}

fn run_order(
    workload: &Workload,
    fixture: &DisposableStore,
    sender_closes_first: bool,
) -> Result<Vec<Metric>> {
    let store = RedbStore::open(fixture.path()).context("opening later-owner matrix store")?;
    let mut core = EngineCore::new(store, 8);
    let handle = RelayHandle {
        slot: 0,
        generation: 5,
    };
    core.handle(EngineMsg::RelayConnected(
        handle,
        RelaySessionKey::public(workload.relay().clone()),
    ));
    core.handle(EngineMsg::RelayInformationResolved(
        workload.relay().clone(),
        None,
    ));
    let queries = workload.matrix_queries(2, DemandShape::ExactDuplicate)?;
    let sender_effects = core.handle(EngineMsg::Subscribe(queries[0].clone()));
    let sender = observation_id(&sender_effects)?;
    let sender_revisions = concrete_revisions(&sender_effects);
    let (_, admitted) = flush_phase_capture(&mut core, "later_owner_setup", "later_owner_setup");
    let requests = wire_requests(&admitted);
    ensure!(requests.len() == 1);
    let accepted = accept_requests(&mut core, &requests, 5);
    let accepted_witnesses = crate::execution::relay_request_witnesses(&accepted);
    ensure!(
        accepted_witnesses.len() == 1
            && accepted_witnesses
                .iter()
                .all(|witness| witness.observation == sender),
        "the first accepted REQ must initially target only its sender"
    );

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let attached_effects = samples.record(|| core.handle(EngineMsg::Subscribe(queries[1].clone())));
    let later = observation_id(&attached_effects)?;
    let later_revisions = concrete_revisions(&attached_effects);
    ensure!(sender != later);
    let mut counts = EffectCounts::default();
    counts.add(&attached_effects);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let order = if sender_closes_first {
        "sender_first"
    } else {
        "later_owner_first"
    };
    let label = format!("exact_active_attach:{order}");
    let attach = apply_core_work(
        &core,
        counts.apply(
            Metric::new("matrix", "later_owner_attach", &label, elapsed, samples)
                .cpu(cpu)
                .count("observations", 2)
                .count("unique_observation_ids", 2),
        ),
    );
    ensure!(
        attach.counts["wire_ops"] == 0
            && attach.counts["router_compiles"] == 0
            && attach.counts["active_physical_requests"] == 1
            && attach.counts["request_target_refs"] == 2,
        "later exact owner did not attach to the immutable active request"
    );
    let flush = core.handle(EngineMsg::FlushWireAdmission(nostr::Timestamp::from(0u64)));
    ensure!(
        flush.is_empty(),
        "later exact attach left pending wire work"
    );

    let (first, survivor, revisions) = if sender_closes_first {
        (sender, later, &later_revisions)
    } else {
        (later, sender, &sender_revisions)
    };
    let (mut first_close, _) = close_phase_capture(&mut core, &[first], &[0], &label);
    first_close.phase = "later_owner_nonfinal_close";
    ensure!(
        first_close.counts["wire_closes"] == 0
            && first_close.counts["active_physical_requests"] == 1
            && first_close.counts["active_observations"] == 1
            && first_close.counts["request_target_refs"] == 1,
        "non-final exact owner closed the incumbent request"
    );

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let settled_effects = samples.record(|| eose_request(&mut core, &requests[0], 0, 5));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let settled = request_settled_witnesses(&settled_effects);
    ensure!(
        settled.len() == 1 && settled[0].observation == survivor,
        "incumbent EOSE did not settle exactly the surviving active owner"
    );
    ensure!(
        revisions.iter().any(|(observation, path, revision, _)| {
            *observation == survivor
                && path == &settled[0].path
                && *revision == settled[0].filter_revision
        }),
        "settlement did not use the surviving owner's current filter revision"
    );
    let mut settled_counts = EffectCounts::default();
    settled_counts.add(&settled_effects);
    let eose = apply_core_work(
        &core,
        settled_counts.apply(
            Metric::new("matrix", "later_owner_eose", &label, elapsed, samples)
                .cpu(cpu)
                .count("request_settled_facts", settled.len() as u64)
                .count("settled_later_owner", u64::from(survivor == later)),
        ),
    );
    ensure!(
        eose.counts["active_execution_owners"] == 0 && eose.counts["live_wire_owners"] == 1,
        "EOSE did not retire only execution evidence while retaining live wire ownership"
    );

    let (mut final_close, _) = close_phase_capture(&mut core, &[survivor], &[0], &label);
    final_close.phase = "later_owner_final_close";
    ensure!(final_close.counts["wire_closes"] == 1);
    ensure_zero_census(&final_close)?;
    Ok(vec![attach, first_close, eose, final_close])
}

#[cfg(test)]
#[path = "later_active_owner/tests.rs"]
mod tests;
