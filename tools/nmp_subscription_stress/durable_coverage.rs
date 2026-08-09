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
    reopen_connected, row_ids,
};
use crate::workload::Workload;

const WRITTEN_THROUGH: u64 = 1_800_100_000;
const MAX_AGE: u64 = 3_600;
const GENERATION: u64 = 91;

pub(crate) fn run(workload: &Workload) -> Result<Vec<Metric>> {
    let root = tempfile::Builder::new()
        .prefix("mosaico-durable-coverage-")
        .tempdir()
        .context("creating durable coverage root")?;
    let path = root.path().join("nmp-written.redb");
    let event = metadata(workload, WRITTEN_THROUGH - 10, "durable")?;

    let mut writer = open_connected(&path, workload, GENERATION)?;
    writer.handle(EngineMsg::Tick(Timestamp::from(WRITTEN_THROUGH)));
    reset(&mut writer);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let (writer_id, request, mut effects) = begin_live(
        &mut writer,
        workload,
        GENERATION,
        Timestamp::from(WRITTEN_THROUGH),
    )?;
    let delivered = samples.record(|| ingest(&mut writer, &request, event.clone(), GENERATION));
    ensure!(added(&delivered, event.id) == 1);
    effects.extend(delivered);
    let settled = samples.record(|| eose_request(&mut writer, &request, 0, GENERATION));
    let settlements = request_settled_witnesses(&settled);
    ensure!(settlements.len() == 1 && settlements[0].observation == writer_id);
    effects.extend(settled);
    let coverage = writer
        .get_coverage(&workload.profile_atom(0), workload.relay())?
        .context("accepted EOSE did not write coverage")?;
    ensure!(coverage.through == Timestamp::from(WRITTEN_THROUGH));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let write = measured(
        &writer,
        "durable_coverage_write",
        "accepted_eose",
        effects,
        elapsed,
        cpu,
        samples,
    )
    .count("event_ingested", 1)
    .count("request_settled_facts", 1)
    .count("coverage_through", coverage.through.as_secs())
    .contract_status(true)
    .note(
        "coverage is caused by signed relay EVENT plus accepted REQ/EOSE, never fixture insertion",
    );
    let writer_close = close(&mut writer, writer_id, "durable_writer_close")?;
    drop(writer);

    let mut fresh = reopen_connected(&path, workload, GENERATION + 1)?;
    fresh.handle(EngineMsg::Tick(Timestamp::from(WRITTEN_THROUGH + 60)));
    reset(&mut fresh);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let query = workload.profile_query_with_freshness(0, Freshness::MaxAge { seconds: MAX_AGE })?;
    let opened = samples.record(|| fresh.handle(EngineMsg::Subscribe(query)));
    let fresh_id = observation_id(&opened)?;
    let admitted = samples.record(|| {
        fresh.handle(EngineMsg::FlushWireAdmission(Timestamp::from(
            WRITTEN_THROUGH + 60,
        )))
    });
    ensure!(row_ids(&opened) == vec![event.id]);
    ensure!(wire_requests(&admitted).is_empty());
    ensure!(reconciled_through(&opened, workload) == Some(WRITTEN_THROUGH));
    let mut fresh_effects = opened;
    fresh_effects.extend(admitted);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let reopened_fresh = measured(
        &fresh,
        "durable_coverage_reopen_fresh",
        "max_age_satisfied",
        fresh_effects,
        elapsed,
        cpu,
        samples,
    )
    .count("reopened_rows", 1)
    .count("expected_wire_reqs", 0)
    .count("persisted_reconciled_through", WRITTEN_THROUGH)
    .contract_status(true)
    .note("new EngineCore and reopened Redb reuse only NMP-written coverage; no socket provenance claim");
    ensure!(
        reopened_fresh.counts["coverage_reads"] == 1,
        "reopened MaxAge must consult one exact coverage row"
    );
    let fresh_close = close(&mut fresh, fresh_id, "durable_fresh_close")?;
    drop(fresh);

    let mut stale = reopen_connected(&path, workload, GENERATION + 2)?;
    stale.handle(EngineMsg::Tick(Timestamp::from(
        WRITTEN_THROUGH + MAX_AGE + 1,
    )));
    reset(&mut stale);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let query = workload.profile_query_with_freshness(0, Freshness::MaxAge { seconds: MAX_AGE })?;
    let opened = samples.record(|| stale.handle(EngineMsg::Subscribe(query)));
    let stale_id = observation_id(&opened)?;
    let admitted = samples.record(|| {
        stale.handle(EngineMsg::FlushWireAdmission(Timestamp::from(
            WRITTEN_THROUGH + MAX_AGE + 1,
        )))
    });
    let requests = wire_requests(&admitted);
    ensure!(row_ids(&opened) == vec![event.id]);
    ensure!(requests.len() == 1 && requests[0].filter == request.filter);
    ensure!(reconciled_through(&opened, workload) == Some(WRITTEN_THROUGH));
    let mut stale_effects = opened;
    stale_effects.extend(admitted);
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let reopened_stale = measured(
        &stale,
        "durable_coverage_reopen_stale",
        "max_age_expired",
        stale_effects,
        elapsed,
        cpu,
        samples,
    )
    .count("reopened_rows", 1)
    .count("expected_wire_reqs", 1)
    .count("ordinary_live_filter_match", 1)
    .contract_status(true)
    .note("expired persisted coverage retains the row but emits the exact ordinary Live request");
    let stale_close = close(&mut stale, stale_id, "durable_stale_close")?;

    Ok(vec![
        write,
        writer_close,
        reopened_fresh,
        fresh_close,
        reopened_stale,
        stale_close,
    ])
}
