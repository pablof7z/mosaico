use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::{EngineCore, EngineMsg, ObservationId};
use nmp::Freshness;
use nmp_store::{CoverageInterval, EventStore, RedbStore};
use nostr::Timestamp;

use crate::core::EffectCounts;
use crate::core_metrics::{apply_core_work, reset};
use crate::execution::{accept_requests, relay_request_witnesses, wire_requests};
use crate::lifecycle::{close_phase_capture, ensure_unique, flush_phase_capture, observation_id};
use crate::matrix_oracle::ensure_zero_census;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

const NOW: u64 = 1_800_000_000;
const MAX_AGE_SECONDS: u64 = 3_600;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Live,
    CacheOnly,
    MaxAgeCurrent,
    MaxAgeStale,
    MaxAgeMissing,
}

impl Mode {
    fn at(index: usize) -> Self {
        match index % 5 {
            0 => Self::Live,
            1 => Self::CacheOnly,
            2 => Self::MaxAgeCurrent,
            3 => Self::MaxAgeStale,
            _ => Self::MaxAgeMissing,
        }
    }

    fn freshness(self) -> Freshness {
        match self {
            Self::Live => Freshness::Live,
            Self::CacheOnly => Freshness::CacheOnly,
            Self::MaxAgeCurrent | Self::MaxAgeStale | Self::MaxAgeMissing => Freshness::MaxAge {
                seconds: MAX_AGE_SECONDS,
            },
        }
    }

    fn owns_wire(self) -> bool {
        matches!(self, Self::Live | Self::MaxAgeStale | Self::MaxAgeMissing)
    }
}

pub(crate) fn run(
    workload: &Workload,
    fixture: &DisposableStore,
    count: usize,
) -> Result<Vec<Metric>> {
    ensure!(
        count >= 5,
        "mixed freshness needs all five deterministic classes"
    );
    let mut store = RedbStore::open(fixture.path()).context("opening mixed freshness store")?;
    seed_coverage(&mut store, workload, count)?;
    let mut core = EngineCore::new(store, count.max(8));
    core.handle(EngineMsg::Tick(Timestamp::from(NOW)));
    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let mut counts = EffectCounts::default();
    let mut wire_ids = Vec::new();
    let mut local_ids = Vec::new();
    let mut all_ids = Vec::new();
    let mut mode_counts = [0u64; 5];
    for index in 0..count {
        let mode = Mode::at(index);
        mode_counts[index % 5] += 1;
        let query = workload.profile_query_with_freshness(index, mode.freshness())?;
        let effects = samples.record(|| core.handle(EngineMsg::Subscribe(query)));
        let id = observation_id(&effects)?;
        if mode.owns_wire() {
            wire_ids.push(id);
        } else {
            local_ids.push(id);
        }
        all_ids.push(id);
        counts.add(&effects);
    }
    ensure_unique(&all_ids)?;
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let label = format!("n={count}:mixed_freshness");
    let open = apply_core_work(
        &core,
        counts.apply(
            Metric::new("matrix", "mixed_freshness_open", &label, elapsed, samples)
                .cpu(cpu)
                .count("observations", count as u64)
                .count("unique_observation_ids", count as u64)
                .count("live", mode_counts[0])
                .count("cache_only", mode_counts[1])
                .count("max_age_current", mode_counts[2])
                .count("max_age_stale", mode_counts[3])
                .count("max_age_missing", mode_counts[4])
                .count("wire_owners", wire_ids.len() as u64)
                .count("local_only_owners", local_ids.len() as u64),
        ),
    );
    let max_age_count = mode_counts[2] + mode_counts[3] + mode_counts[4];
    ensure!(
        open.counts["coverage_reads"] == max_age_count
            && open.counts["request_target_handles"] == count as u64
            && open.counts["request_target_demand_keys"] == wire_ids.len() as u64
            && open.counts["request_target_edges"] == wire_ids.len() as u64
            && open.counts["request_target_refs"] == wire_ids.len() as u64,
        "opening-time freshness did not isolate exact wire-active occurrences (reads={}, handles={}, demand_keys={}, edges={}, refs={}, max_age={}, wire={})",
        open.counts["coverage_reads"],
        open.counts["request_target_handles"],
        open.counts["request_target_demand_keys"],
        open.counts["request_target_edges"],
        open.counts["request_target_refs"],
        max_age_count,
        wire_ids.len(),
    );

    let (admission, admitted) = flush_phase_capture(&mut core, &label, "mixed_freshness_admission");
    let requests = wire_requests(&admitted);
    ensure!(!requests.is_empty());
    ensure!(
        admission.counts["request_target_demand_keys_touched"] == 0
            && admission.counts["request_target_candidates_examined"] == 0,
        "planning must not project mixed request execution before local acceptance"
    );

    reset(&mut core);
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let accepted = samples.record(|| accept_requests(&mut core, &requests, 3));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let witnesses = relay_request_witnesses(&accepted);
    ensure!(
        witnesses
            .iter()
            .map(|witness| witness.observation)
            .collect::<BTreeSet<_>>()
            == wire_ids.iter().copied().collect()
            && witnesses.len() == wire_ids.len(),
        "RelayRequest evidence crossed the mixed freshness ownership boundary"
    );
    let mut accepted_counts = EffectCounts::default();
    accepted_counts.add(&accepted);
    let handoff = apply_core_work(
        &core,
        accepted_counts.apply(
            Metric::new(
                "matrix",
                "mixed_freshness_handoff",
                &label,
                elapsed,
                samples,
            )
            .cpu(cpu)
            .count("relay_request_facts", witnesses.len() as u64),
        ),
    );
    ensure!(
        handoff.counts["request_target_demand_keys_touched"] == wire_ids.len() as u64
            && handoff.counts["request_target_candidates_examined"] == wire_ids.len() as u64,
        "accepted mixed requests visited local-only freshness targets"
    );

    let mut local_close = close_group(&mut core, &local_ids, &label);
    local_close.phase = "mixed_local_only_close";
    ensure!(
        local_close.counts["wire_closes"] == 0
            && local_close.counts["active_physical_requests"] == requests.len() as u64,
        "CacheOnly or satisfied MaxAge close changed wire lifecycle"
    );
    let mut wire_close = close_group(&mut core, &wire_ids, &label);
    wire_close.phase = "mixed_wire_owner_close";
    ensure!(wire_close.counts["wire_closes"] == requests.len() as u64);
    ensure_zero_census(&wire_close)?;
    Ok(vec![open, admission, handoff, local_close, wire_close])
}

fn seed_coverage(store: &mut RedbStore, workload: &Workload, count: usize) -> Result<()> {
    let claims = (0..count)
        .filter_map(|index| match Mode::at(index) {
            Mode::MaxAgeCurrent => Some((index, NOW - 60)),
            Mode::MaxAgeStale => Some((index, NOW - MAX_AGE_SECONDS - 1)),
            _ => None,
        })
        .map(|(index, through)| {
            (
                workload.profile_atom(index),
                workload.relay().clone(),
                CoverageInterval::new(Timestamp::from(0), Timestamp::from(through)),
            )
        })
        .collect::<Vec<_>>();
    store
        .record_coverage(&claims)
        .context("seeding mixed MaxAge coverage")?;
    Ok(())
}

fn close_group(core: &mut EngineCore<RedbStore>, ids: &[ObservationId], label: &str) -> Metric {
    let order = (0..ids.len()).collect::<Vec<_>>();
    close_phase_capture(core, ids, &order, label).0
}

#[cfg(test)]
#[path = "freshness_matrix/tests.rs"]
mod tests;
