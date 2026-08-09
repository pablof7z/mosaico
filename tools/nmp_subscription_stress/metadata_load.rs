use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use nmp::{AccessContext, SourceAuthority};
use nmp_grammar::{ConcreteFilter, ContextualAtom};
use nmp_router::{FixtureRoutingFacts, FullMetadataWork, Router, RuleRegistry, WireOp};
use nostr::RelayUrl;

use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};

const HISTORICAL_ENTRIES: usize = 10_000;

pub(crate) fn run(include_ten_thousand: bool) -> Result<Vec<Metric>> {
    let mut metrics = Vec::new();
    if include_ten_thousand {
        metrics.push(delta_metadata()?);
        metrics.push(full_historical_metadata()?);
    }
    metrics.push(unavailable_post_eose_retry_load());
    Ok(metrics)
}

fn delta_metadata() -> Result<Metric> {
    let relay = RelayUrl::parse("wss://nmp-stress-metadata-delta.invalid")?;
    let facts = FixtureRoutingFacts::new();
    let mut incumbent_atoms = BTreeSet::new();
    let mut wide_kinds = BTreeSet::new();
    for kind in 10_000..20_000 {
        incumbent_atoms.insert(pinned(&relay, [kind]));
        wide_kinds.insert(kind);
    }
    let wide = pinned(&relay, wide_kinds);
    let mut router = Router::new(RuleRegistry::default_widen_only());
    let setup = router.admit(&incumbent_atoms, &facts, 20);
    ensure!(setup.wire.ops.len() == 1);
    let setup_requests = router.plan().reqs.values().flatten().collect::<Vec<_>>();
    ensure!(setup_requests.len() == 1);
    ensure!(setup_requests[0].coverage_claims.len() == HISTORICAL_ENTRIES);

    router.reset_admission_work();
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let outcome = samples.record(|| router.admit(&BTreeSet::from([wide.clone()]), &facts, 20));
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let work = router.admission_work();
    let satisfied = outcome.wire.ops.is_empty()
        && outcome.request_metadata_updates.len() == 1
        && outcome.request_metadata_updates[0]
            .added_owner_demands
            .len()
            == 1
        && outcome.request_metadata_updates[0]
            .added_coverage_claims
            .len()
            == 1
        && work.metadata_entries_examined == 3;
    ensure!(
        satisfied,
        "delta metadata admission revisited incumbent history"
    );
    let metric = Metric::new(
        "internal_control",
        "metadata_delta_over_10k",
        "one_incumbent_request",
        elapsed,
        samples,
    )
    .cpu(cpu)
    .count("historical_claims", HISTORICAL_ENTRIES as u64)
    .count("metadata_entries_examined", work.metadata_entries_examined)
    .count(
        "request_metadata_updates",
        outcome.request_metadata_updates.len() as u64,
    )
    .count("wire_ops", wire_ops(&outcome.wire) as u64)
    .count("contract_satisfied", u64::from(satisfied))
    .contract_status(satisfied)
    .note(
        "one exact metadata delta must examine only candidate entries, never 10k incumbent claims",
    );

    router.withdraw(incumbent_atoms, 20);
    router.withdraw([wide], 20);
    ensure!(router.ownership_census() == Default::default());
    Ok(metric)
}

fn full_historical_metadata() -> Result<Metric> {
    let facts = FixtureRoutingFacts::new();
    let mut historical = BTreeSet::new();
    for index in 0..HISTORICAL_ENTRIES {
        let relay = RelayUrl::parse(&format!("wss://nmp-stress-historical-{index:05}.invalid"))?;
        historical.insert(pinned(&relay, [((index % 50_000) + 1) as u16]));
    }
    let current = historical
        .first()
        .cloned()
        .expect("the fixed historical corpus is nonempty");
    let mut router = Router::new(RuleRegistry::default_widen_only());
    router.compile(&historical, &facts, HISTORICAL_ENTRIES);
    ensure!(router.plan().reqs.len() == HISTORICAL_ENTRIES);

    router.reset_full_metadata_work();
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut samples = Samples::default();
    let outcome = samples.record(|| {
        router.compile(
            &BTreeSet::from([current.clone()]),
            &facts,
            HISTORICAL_ENTRIES,
        )
    });
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let FullMetadataWork {
        requests_probed,
        candidate_entries_examined,
        owner_edges_visited,
        assignment_edges_visited,
        provenance_author_edges_visited,
        diagnostic_provenance_edges_visited,
    } = router.full_metadata_work();
    let closes = outcome
        .wire
        .ops
        .iter()
        .flat_map(|(_, operations)| operations)
        .filter(|operation| matches!(operation, WireOp::Close(_)))
        .count();
    let satisfied = closes == HISTORICAL_ENTRIES - 1
        && requests_probed == 1
        && candidate_entries_examined == 3
        && owner_edges_visited == 1
        && assignment_edges_visited == 0
        && provenance_author_edges_visited == 0
        && diagnostic_provenance_edges_visited == 0;
    ensure!(
        satisfied,
        "full compile rescanned metadata for retired historical requests"
    );
    let metric = Metric::new(
        "internal_control",
        "full_historical_metadata",
        "one_survivor_of_10k",
        elapsed,
        samples,
    )
    .cpu(cpu)
    .count("historical_requests", HISTORICAL_ENTRIES as u64)
    .count("wire_closes", closes as u64)
    .count("metadata_requests_probed", requests_probed)
    .count(
        "metadata_candidate_entries_examined",
        candidate_entries_examined,
    )
    .count("metadata_owner_edges_visited", owner_edges_visited)
    .count("metadata_assignment_edges_visited", assignment_edges_visited)
    .count(
        "metadata_provenance_author_edges_visited",
        provenance_author_edges_visited,
    )
    .count(
        "metadata_diagnostic_provenance_edges_visited",
        diagnostic_provenance_edges_visited,
    )
    .count("contract_satisfied", u64::from(satisfied))
    .contract_status(satisfied)
    .note("whole-plan recovery may close history, but metadata reconciliation probes only the surviving request");

    router.withdraw([current], HISTORICAL_ENTRIES);
    ensure!(router.ownership_census() == Default::default());
    Ok(metric)
}

fn unavailable_post_eose_retry_load() -> Metric {
    Metric::new(
        "internal_control",
        "post_eose_retry_load",
        "fault_injection",
        Duration::ZERO,
        Samples::default(),
    )
    .count("target_generations", 1_000)
    .count("public_fault_seam_available", 0)
    .unavailable()
    .note("unavailable: the public bench door exposes counters but no coverage-write fault injector or metadata-transfer driver")
}

fn pinned(relay: &RelayUrl, kinds: impl IntoIterator<Item = u16>) -> ContextualAtom {
    ContextualAtom {
        filter: ConcreteFilter {
            kinds: Some(kinds.into_iter().collect()),
            ..ConcreteFilter::default()
        },
        source: SourceAuthority::Pinned(BTreeSet::from([relay.clone()])),
        access: AccessContext::Public,
        routing_evidence: BTreeSet::new(),
    }
}

fn wire_ops(delta: &nmp_router::WireDelta) -> usize {
    delta
        .ops
        .iter()
        .map(|(_, operations)| operations.len())
        .sum()
}
