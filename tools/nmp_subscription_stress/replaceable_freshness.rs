use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::EngineMsg;
use nmp::Freshness;
use nostr::Timestamp;

use crate::core_metrics::reset;
use crate::execution::{eose_request, request_settled_witnesses, wire_requests};
use crate::lifecycle::observation_id;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::profile_control::{
    added, begin_live, close, ingest, measured, metadata, open_connected, reconciled_through,
    removed, row_ids,
};
use crate::workload::Workload;

const WRITTEN_THROUGH: u64 = 1_800_000_000;
const MAX_AGE: u64 = 3_600;
const GENERATION: u64 = 81;

pub(crate) fn run(workload: &Workload) -> Result<Vec<Metric>> {
    let root = tempfile::Builder::new()
        .prefix("mosaico-replaceable-freshness-")
        .tempdir()
        .context("creating replaceable freshness root")?;
    let path = root.path().join("replaceable.redb");
    let mut core = open_connected(&path, workload, GENERATION)?;
    core.handle(EngineMsg::Tick(Timestamp::from(WRITTEN_THROUGH)));

    let older = metadata(workload, 100, "older")?;
    let newer = metadata(workload, 200, "newer")?;
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let (live_id, request, mut effects) = begin_live(
        &mut core,
        workload,
        GENERATION,
        Timestamp::from(WRITTEN_THROUGH),
    )?;
    let old_effects = samples.record(|| ingest(&mut core, &request, older.clone(), GENERATION));
    let new_effects = samples.record(|| ingest(&mut core, &request, newer.clone(), GENERATION));
    let settled = samples.record(|| eose_request(&mut core, &request, 0, GENERATION));
    ensure!(added(&old_effects, older.id) == 1);
    ensure!(removed(&new_effects, older.id) == 1 && added(&new_effects, newer.id) == 1);
    let settlements = request_settled_witnesses(&settled);
    ensure!(settlements.len() == 1 && settlements[0].observation == live_id);
    effects.extend(old_effects);
    effects.extend(new_effects);
    effects.extend(settled);
    let coverage = core
        .get_coverage(&workload.profile_atom(0), workload.relay())?
        .context("EOSE did not write profile coverage")?;
    ensure!(coverage.through == Timestamp::from(WRITTEN_THROUGH));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let setup = measured(
        &core,
        "replaceable_ingest_eose",
        "older_then_newer",
        effects,
        elapsed,
        cpu,
        samples,
    )
    .count("older_added", 1)
    .count("older_removed", 1)
    .count("newer_added", 1)
    .count("coverage_through", coverage.through.as_secs())
    .contract_status(true)
    .note("signed relay EVENTs plus accepted REQ/EOSE; reducer/store proof, not socket provenance");
    let first_close = close(&mut core, live_id, "replaceable_live_close")?;

    core.handle(EngineMsg::Tick(Timestamp::from(WRITTEN_THROUGH + 60)));
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let query = workload.profile_query_with_freshness(0, Freshness::MaxAge { seconds: MAX_AGE })?;
    let opened = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
    let fresh_id = observation_id(&opened)?;
    let admitted = samples.record(|| {
        core.handle(EngineMsg::FlushWireAdmission(Timestamp::from(
            WRITTEN_THROUGH + 60,
        )))
    });
    ensure!(row_ids(&opened) == vec![newer.id]);
    ensure!(wire_requests(&admitted).is_empty());
    ensure!(reconciled_through(&opened, workload) == Some(WRITTEN_THROUGH));
    let mut fresh_effects = opened;
    fresh_effects.extend(admitted);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let fresh = measured(
        &core,
        "replaceable_recent_max_age",
        "newer_winner",
        fresh_effects,
        elapsed,
        cpu,
        samples,
    )
    .count("selected_newer", 1)
    .count("selected_older", 0)
    .count("expected_wire_reqs", 0)
    .contract_status(true)
    .note("document replacement and coverage freshness are asserted as independent outputs");
    let fresh_close = close(&mut core, fresh_id, "replaceable_fresh_close")?;

    let future = metadata(workload, WRITTEN_THROUGH + 1_000_000, "future")?;
    core.handle(EngineMsg::Tick(Timestamp::from(WRITTEN_THROUGH + 60)));
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let (future_live_id, future_request, mut future_effects) = begin_live(
        &mut core,
        workload,
        GENERATION,
        Timestamp::from(WRITTEN_THROUGH + 60),
    )?;
    let delivered =
        samples.record(|| ingest(&mut core, &future_request, future.clone(), GENERATION));
    ensure!(removed(&delivered, newer.id) == 1 && added(&delivered, future.id) == 1);
    future_effects.extend(delivered);
    future_effects
        .extend(samples.record(|| eose_request(&mut core, &future_request, 0, GENERATION)));
    let future_coverage = core
        .get_coverage(&workload.profile_atom(0), workload.relay())?
        .context("future EOSE did not write coverage")?;
    ensure!(future_coverage.through == Timestamp::from(WRITTEN_THROUGH + 60));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let future_ingest = measured(
        &core,
        "replaceable_future_ingest",
        "future_winner",
        future_effects,
        elapsed,
        cpu,
        samples,
    )
    .count("future_added", 1)
    .count("newer_removed", 1)
    .count("coverage_through", future_coverage.through.as_secs())
    .count("future_created_at", future.created_at.as_secs())
    .contract_status(true)
    .note("future created_at wins replacement but EOSE coverage remains reducer-clock bounded");
    let future_live_close = close(&mut core, future_live_id, "replaceable_future_live_close")?;

    core.handle(EngineMsg::Tick(Timestamp::from(
        WRITTEN_THROUGH + 60 + MAX_AGE + 1,
    )));
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let query = workload.profile_query_with_freshness(0, Freshness::MaxAge { seconds: MAX_AGE })?;
    let opened = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
    let stale_id = observation_id(&opened)?;
    let admitted = samples.record(|| {
        core.handle(EngineMsg::FlushWireAdmission(Timestamp::from(
            WRITTEN_THROUGH + 60 + MAX_AGE + 1,
        )))
    });
    let stale_requests = wire_requests(&admitted);
    ensure!(row_ids(&opened) == vec![future.id]);
    ensure!(stale_requests.len() == 1);
    ensure!(stale_requests[0].filter == future_request.filter);
    ensure!(reconciled_through(&admitted, workload) == Some(WRITTEN_THROUGH + 60));
    let mut stale_effects = opened;
    stale_effects.extend(admitted);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let stale = measured(
        &core,
        "replaceable_stale_max_age",
        "future_winner",
        stale_effects,
        elapsed,
        cpu,
        samples,
    )
    .count("selected_future", 1)
    .count("expected_wire_reqs", 1)
    .count("future_age_satisfied_freshness", 0)
    .contract_status(true)
    .note("future document remains the row winner but stale EOSE coverage emits ordinary Live wire work");
    let stale_close = close(&mut core, stale_id, "replaceable_stale_close")?;

    Ok(vec![
        setup,
        first_close,
        fresh,
        fresh_close,
        future_ingest,
        future_live_close,
        stale,
        stale_close,
    ])
}

#[cfg(test)]
#[path = "replaceable_freshness/tests.rs"]
mod tests;
