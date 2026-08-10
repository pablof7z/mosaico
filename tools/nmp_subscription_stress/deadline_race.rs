use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{RelayAdmissionPolicy, RowDelta};
use nmp::mechanism::runtime::{EngineThread, ObservationOwnershipCensus};
use nmp_store::{EventStore, MemoryStore, RelayObserved};
use nmp_transport::PoolConfig;
use nostr::{EventBuilder, Kind, Tag, Timestamp};

use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::workload::Workload;

const BASE: u64 = 1_800_200_000;

pub(crate) fn run(workload: &Workload) -> Result<Metric> {
    let expiry = Timestamp::from(BASE + 1);
    let event = EventBuilder::new(Kind::Metadata, r#"{"name":"deadline-race"}"#)
        .tag(Tag::parse(["expiration", &expiry.as_secs().to_string()])?)
        .custom_created_at(Timestamp::from(BASE))
        .sign_with_keys(&workload.identities[0])
        .context("signing deadline-race profile")?;
    let event_id = event.id;
    let mut store = MemoryStore::new();
    store.insert(
        event,
        RelayObserved::new(workload.relay().clone(), Timestamp::from(BASE)),
    )?;
    let (thread, handle) = EngineThread::spawn(
        store,
        8,
        PoolConfig::default(),
        RelayAdmissionPolicy::default(),
    )?;
    thread.clock().set(Timestamp::from(BASE));
    let (observation, rows) = handle.subscribe(workload.profile_query(0)?)?;
    let (initial, _, _) = rows
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| anyhow::anyhow!("opening deadline row was not delivered: {error}"))?;
    ensure!(initial
        .iter()
        .any(|delta| matches!(delta, RowDelta::Added(row) if row.event.id == event_id)));

    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let hold = handle.bench_hold_due_deadline_command(expiry);
    let (expired, _, _) = rows
        .recv_timeout(Duration::from_millis(100))
        .map_err(|error| {
            anyhow::anyhow!("due expiration did not beat the simultaneously-ready command: {error}")
        })?;
    ensure!(expired
        .iter()
        .any(|delta| matches!(delta, RowDelta::Removed(id) if *id == event_id)));
    drop(hold);
    handle.unsubscribe(observation);
    ensure!(
        handle.observation_ownership_census() == ObservationOwnershipCensus::default(),
        "deadline-race observation did not tear down"
    );
    handle.shutdown();
    thread.join();
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let mut samples = Samples::default();
    samples.push(elapsed);
    Ok(Metric::new(
        "runtime_control",
        "due_deadline_command_race",
        "deadline_first",
        elapsed,
        samples,
    )
    .cpu(cpu)
    .count("expired_rows_before_command", 1)
    .count("final_ownership_census", 0)
    .contract_status(true)
    .note("a deterministic runtime probe holds the simultaneous command after the due expiration has already published"))
}

#[cfg(test)]
#[path = "deadline_race/tests.rs"]
mod tests;
