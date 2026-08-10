use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg, ObservationId};
use nmp_grammar::RelaySessionKey;
use nmp_store::RedbStore;
use nmp_transport::RelayHandle;

use crate::execution::{accept_requests, wire_requests};
use crate::lifecycle::{close_phase_capture, flush_phase_capture, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::Metric;
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run(workload: &Workload, fixture: &DisposableStore) -> Result<Vec<Metric>> {
    let mut metrics = pending_grouping(workload, fixture)?;
    metrics.extend(compatible_later_executes(workload, fixture)?);
    metrics.extend(semantic_superset(workload, fixture)?);
    metrics.extend(partial_residual(workload, fixture)?);
    Ok(metrics)
}

fn pending_grouping(workload: &Workload, fixture: &DisposableStore) -> Result<Vec<Metric>> {
    let mut core = core(fixture, workload, 1)?;
    let first = workload.live_authors_query([0], [0])?;
    let second = workload.live_authors_query([0], [1])?;
    let ids = [
        observation_id(&core.handle(EngineMsg::Subscribe(first)))?,
        observation_id(&core.handle(EngineMsg::Subscribe(second)))?,
    ];
    let (mut admission, effects) =
        flush_phase_capture(&mut core, "pending_grouping", "semantic_pending_grouping");
    let requests = wire_requests(&effects);
    let expected = workload.concrete_authors_filter([0], [0, 1]);
    let satisfied = requests.len() == 1
        && requests[0].filter == expected
        && admission.counts["wire_closes"] == 0;
    admission = admission
        .count("contract_satisfied", u64::from(satisfied))
        .count("expected_wire_reqs", 1)
        .contract_status(satisfied)
        .note("two compatible observations waiting in the same cohort must become one grouped REQ");
    ensure!(satisfied, "pending compatible demand did not group exactly");
    let teardown = close_all(&mut core, &ids, "pending_grouping")?;
    Ok(vec![admission, teardown])
}

fn compatible_later_executes(
    workload: &Workload,
    fixture: &DisposableStore,
) -> Result<Vec<Metric>> {
    let mut core = core(fixture, workload, 2)?;
    let first =
        observation_id(&core.handle(EngineMsg::Subscribe(workload.live_authors_query([0], [0])?)))?;
    let (_, first_effects) =
        flush_phase_capture(&mut core, "compatible_later", "compatible_later_incumbent");
    let first_requests = wire_requests(&first_effects);
    ensure!(first_requests.len() == 1);
    accept_requests(&mut core, &first_requests, 2);

    let later =
        observation_id(&core.handle(EngineMsg::Subscribe(workload.live_authors_query([0], [1])?)))?;
    let (mut admission, effects) =
        flush_phase_capture(&mut core, "compatible_later", "compatible_later_executes");
    let requests = wire_requests(&effects);
    let expected = workload.concrete_authors_filter([0], [1]);
    let satisfied = requests.len() == 1
        && requests[0].filter == expected
        && requests[0].sub_id != first_requests[0].sub_id
        && admission.counts["wire_closes"] == 0;
    admission = admission
        .count("contract_satisfied", u64::from(satisfied))
        .count("expected_wire_reqs", 1)
        .contract_status(satisfied)
        .note("later compatible but uncovered demand must execute without replacing the incumbent REQ");
    ensure!(
        satisfied,
        "later compatible uncovered demand did not execute exactly"
    );
    let teardown = close_all(&mut core, &[first, later], "compatible_later")?;
    Ok(vec![admission, teardown])
}

fn semantic_superset(workload: &Workload, fixture: &DisposableStore) -> Result<Vec<Metric>> {
    let mut core = core(fixture, workload, 3)?;
    let incumbent = observation_id(&core.handle(EngineMsg::Subscribe(
        workload.live_authors_query([0, 1], [0, 1])?,
    )))?;
    let (_, incumbent_effects) = flush_phase_capture(
        &mut core,
        "semantic_superset",
        "semantic_superset_incumbent",
    );
    let incumbent_requests = wire_requests(&incumbent_effects);
    ensure!(incumbent_requests.len() == 1);
    accept_requests(&mut core, &incumbent_requests, 3);

    let later = observation_id(&core.handle(EngineMsg::Subscribe(
        workload.live_authors_query([1], [0, 1])?,
    )))?;
    let (mut admission, effects) =
        flush_phase_capture(&mut core, "semantic_superset", "semantic_superset_attach");
    let requests = wire_requests(&effects);
    let satisfied = requests.is_empty()
        && admission.counts["wire_closes"] == 0
        && admission.counts["active_physical_requests"] == 1
        && admission.counts["request_target_refs"] == 2;
    admission = admission
        .count("contract_satisfied", u64::from(satisfied))
        .count("expected_wire_reqs", 0)
        .contract_status(satisfied)
        .note("known red: an active semantic superset should attach the later observation locally");

    let (mut release, _) = close_phase_capture(&mut core, &[incumbent], &[0], "semantic_superset");
    release.phase = "semantic_superset_incumbent_release";
    let release_satisfied = satisfied
        && release.counts["wire_closes"] == 0
        && release.counts["active_physical_requests"] == 1;
    release = release
        .count("contract_satisfied", u64::from(release_satisfied))
        .count("expected_wire_closes", 0)
        .contract_status(release_satisfied)
        .note("known red: the surviving semantic owner must keep its incumbent physical coverage alive");
    let teardown = close_all(&mut core, &[later], "semantic_superset")?;
    Ok(vec![admission, release, teardown])
}

fn partial_residual(workload: &Workload, fixture: &DisposableStore) -> Result<Vec<Metric>> {
    let mut core = core(fixture, workload, 4)?;
    let incumbent = observation_id(&core.handle(EngineMsg::Subscribe(
        workload.live_authors_query([0, 1], [0, 1])?,
    )))?;
    let (_, incumbent_effects) =
        flush_phase_capture(&mut core, "partial_residual", "partial_residual_incumbent");
    let incumbent_requests = wire_requests(&incumbent_effects);
    ensure!(incumbent_requests.len() == 1);
    accept_requests(&mut core, &incumbent_requests, 4);

    let later = observation_id(&core.handle(EngineMsg::Subscribe(
        workload.live_authors_query([1], [0, 1, 2])?,
    )))?;
    let (mut admission, effects) =
        flush_phase_capture(&mut core, "partial_residual", "partial_exact_residual");
    let requests = wire_requests(&effects);
    let expected = workload.concrete_authors_filter([1], [2]);
    let full_b = workload.concrete_authors_filter([1], [0, 1, 2]);
    let common_safe_shape = requests.len() == 1
        && admission.counts["wire_closes"] == 0
        && admission.counts["active_physical_requests"] == 2;
    let satisfied = common_safe_shape && requests[0].filter == expected;
    let current_safe_full_b = common_safe_shape && requests[0].filter == full_b;
    ensure!(
        satisfied || current_safe_full_b,
        "partial coverage emitted neither the exact residual nor the complete safe B filter"
    );
    admission = admission
        .count("contract_satisfied", u64::from(satisfied))
        .count("current_safe_full_b", u64::from(current_safe_full_b))
        .count("expected_wire_reqs", 1)
        .count("expected_residual_authors", 1)
        .contract_status(satisfied);
    if current_safe_full_b {
        admission = admission.known_red_safe_full_b();
    }
    admission = admission.note(
        "target: exact [1] x [c] residual; current safe fallback: full [1] x [a,b,c] B request",
    );

    let (mut release, _) = close_phase_capture(&mut core, &[incumbent], &[0], "partial_residual");
    release.phase = "partial_residual_incumbent_release";
    let release_satisfied = satisfied
        && release.counts["wire_closes"] == 0
        && release.counts["active_physical_requests"] == 2;
    let current_safe_release = current_safe_full_b
        && release.counts["wire_closes"] == 1
        && release.counts["active_physical_requests"] == 1;
    ensure!(
        release_satisfied || current_safe_release,
        "incumbent release retained neither target residual ownership nor the safe full-B fallback"
    );
    release = release
        .count("contract_satisfied", u64::from(release_satisfied))
        .count("current_safe_full_b", u64::from(current_safe_release))
        .count("expected_wire_closes", 0)
        .contract_status(release_satisfied);
    if current_safe_release {
        release = release.known_red_safe_full_b();
    }
    release = release
        .note("target: later owner keeps incumbent plus residual alive; current full-B fallback safely closes incumbent");

    let (mut final_release, _) = close_phase_capture(&mut core, &[later], &[0], "partial_residual");
    final_release.phase = "partial_residual_final_release";
    let final_satisfied = satisfied && final_release.counts["wire_closes"] == 2;
    let current_safe_final = current_safe_full_b && final_release.counts["wire_closes"] == 1;
    ensure!(
        final_satisfied || current_safe_final,
        "final release did not retire the target pair or the safe full-B fallback exactly"
    );
    final_release = final_release
        .count("contract_satisfied", u64::from(final_satisfied))
        .count("current_safe_full_b", u64::from(current_safe_final))
        .count("expected_wire_closes", 2)
        .contract_status(final_satisfied);
    if current_safe_final {
        final_release = final_release.known_red_safe_full_b();
    }
    final_release = final_release
        .note("target: final later close tears down incumbent and residual; current full-B fallback tears down B");
    ensure_zero_census(&final_release)?;
    Ok(vec![admission, release, final_release])
}

fn core(
    fixture: &DisposableStore,
    workload: &Workload,
    generation: u64,
) -> Result<EngineCore<RedbStore>> {
    let store = RedbStore::open(fixture.path()).context("opening semantic coverage store")?;
    let mut core = EngineCore::new(store, 8);
    core.handle(EngineMsg::RelayConnected(
        RelayHandle {
            slot: 0,
            generation,
        },
        RelaySessionKey::public(workload.relay().clone()),
    ));
    core.handle(EngineMsg::RelayInformationResolved(
        workload.relay().clone(),
        None,
    ));
    Ok(core)
}

fn close_all(
    core: &mut EngineCore<RedbStore>,
    ids: &[ObservationId],
    label: &str,
) -> Result<Metric> {
    let order: Vec<_> = (0..ids.len()).collect();
    let (mut teardown, _) = close_phase_capture(core, ids, &order, label);
    teardown.phase = "semantic_teardown";
    ensure_zero_census(&teardown)?;
    Ok(teardown)
}

#[cfg(test)]
#[path = "semantic_coverage/tests.rs"]
mod tests;
