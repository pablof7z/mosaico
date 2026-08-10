use std::path::Path;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{Effect, EngineCore, EngineMsg, ObservationId, RowDelta};
use nmp::Freshness;
use nmp_grammar::RelaySessionKey;
use nmp_store::RedbStore;
use nmp_transport::{RelayFrame, RelayHandle};
use nostr::{Event, EventBuilder, EventId, Kind, RelayMessage, SubscriptionId, Timestamp};

use crate::core::EffectCounts;
use crate::core_metrics::apply_core_work;
use crate::execution::{accept_requests, relay_request_witnesses, wire_requests, WireRequest};
use crate::lifecycle::{close_phase_capture, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::{Metric, Samples};
use crate::workload::Workload;

pub(crate) fn open_connected(
    path: &Path,
    workload: &Workload,
    generation: u64,
) -> Result<EngineCore<RedbStore>> {
    connected(path, workload, generation, false)
}

pub(crate) fn reopen_connected(
    path: &Path,
    workload: &Workload,
    generation: u64,
) -> Result<EngineCore<RedbStore>> {
    connected(path, workload, generation, true)
}

fn connected(
    path: &Path,
    workload: &Workload,
    generation: u64,
    recover: bool,
) -> Result<EngineCore<RedbStore>> {
    let store = RedbStore::open(path).context("opening shared profile control store")?;
    let mut core = EngineCore::new(store, 8);
    if recover {
        ensure!(
            core.recover_on_boot().is_empty(),
            "coverage-only restart unexpectedly recovered volatile work"
        );
    }
    let handle = RelayHandle {
        slot: 0,
        generation,
    };
    core.handle(EngineMsg::RelayConnected(
        handle,
        RelaySessionKey::public(workload.relay().clone()),
    ));
    core.handle(EngineMsg::RelayInformationResolved(
        workload.relay().clone(),
        None,
    ));
    Ok(core)
}

pub(crate) fn begin_live(
    core: &mut EngineCore<RedbStore>,
    workload: &Workload,
    generation: u64,
    admission_at: Timestamp,
) -> Result<(ObservationId, WireRequest, Vec<Effect>)> {
    let mut effects = core.handle(EngineMsg::Subscribe(
        workload.profile_query_with_freshness(0, Freshness::Live)?,
    ));
    let id = observation_id(&effects)?;
    let admitted = core.handle(EngineMsg::FlushWireAdmission(admission_at));
    let requests = wire_requests(&admitted);
    ensure!(requests.len() == 1, "Live profile must emit one exact REQ");
    let accepted = accept_requests(core, &requests, generation);
    let witnesses = relay_request_witnesses(&accepted);
    ensure!(witnesses.len() == 1 && witnesses[0].observation == id);
    effects.extend(admitted);
    effects.extend(accepted);
    Ok((id, requests[0].clone(), effects))
}

pub(crate) fn ingest(
    core: &mut EngineCore<RedbStore>,
    request: &WireRequest,
    event: Event,
    generation: u64,
) -> Vec<Effect> {
    core.handle(EngineMsg::RelayFrame(
        RelayHandle {
            slot: 0,
            generation,
        },
        request.session.clone(),
        RelayFrame::from(RelayMessage::event(
            SubscriptionId::new(request.sub_id.1.to_string()),
            event,
        )),
    ))
}

pub(crate) fn metadata(workload: &Workload, created_at: u64, name: &str) -> Result<Event> {
    EventBuilder::new(Kind::Metadata, format!(r#"{{"name":"{name}"}}"#))
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(&workload.identities[0])
        .context("signing profile control metadata")
}

pub(crate) fn added(effects: &[Effect], id: EventId) -> usize {
    row_deltas(effects)
        .filter(|row| matches!(row, RowDelta::Added(value) if value.event.id == id))
        .count()
}

pub(crate) fn removed(effects: &[Effect], id: EventId) -> usize {
    row_deltas(effects)
        .filter(|row| matches!(row, RowDelta::Removed(value) if *value == id))
        .count()
}

pub(crate) fn row_ids(effects: &[Effect]) -> Vec<EventId> {
    row_deltas(effects)
        .filter_map(|row| match row {
            RowDelta::Added(value) => Some(value.event.id),
            _ => None,
        })
        .collect()
}

fn row_deltas(effects: &[Effect]) -> impl Iterator<Item = &RowDelta> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EmitRows(_, rows, _) => Some(rows),
            _ => None,
        })
        .flatten()
}

pub(crate) fn reconciled_through(effects: &[Effect], workload: &Workload) -> Option<u64> {
    effects.iter().rev().find_map(|effect| match effect {
        Effect::EmitRows(_, _, evidence) => evidence
            .iter()
            .flat_map(|item| &item.sources)
            .find(|source| source.relay == *workload.relay())
            .and_then(|source| source.reconciled_through)
            .map(|timestamp| timestamp.as_secs()),
        _ => None,
    })
}

pub(crate) fn measured(
    core: &EngineCore<RedbStore>,
    phase: &'static str,
    topology: &'static str,
    effects: Vec<Effect>,
    elapsed: std::time::Duration,
    cpu: std::time::Duration,
    samples: Samples,
) -> Metric {
    let mut counts = EffectCounts::default();
    counts.add(&effects);
    apply_core_work(
        core,
        counts.apply(Metric::new("matrix", phase, topology, elapsed, samples).cpu(cpu)),
    )
}

pub(crate) fn close(
    core: &mut EngineCore<RedbStore>,
    id: ObservationId,
    phase: &'static str,
) -> Result<Metric> {
    let (mut metric, _) = close_phase_capture(core, &[id], &[0], phase);
    metric.phase = phase;
    ensure_zero_census(&metric)?;
    Ok(metric)
}
