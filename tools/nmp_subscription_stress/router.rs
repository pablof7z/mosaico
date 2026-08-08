use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::Result;
use nmp_grammar::ContextualAtom;
use nmp_router::{diff_plans, FixtureRoutingFacts, Router, RuleRegistry};

use crate::args::{Args, Topology};
use crate::measure::{Metric, Samples};
use crate::workload::Workload;

pub(crate) fn run_router(
    args: &Args,
    workload: &Workload,
    topology: Topology,
) -> Result<Vec<Metric>> {
    let demand = workload.router_atoms(topology)?;
    let facts = FixtureRoutingFacts::new();
    let mut incremental_router = Router::new(RuleRegistry::default_widen_only());
    let mut incremental_demand = BTreeSet::new();
    let incremental_started = Instant::now();
    let mut incremental_samples = Samples::default();
    let mut incremental_wire_ops = 0usize;
    for atom in &demand {
        incremental_demand.insert(atom.clone());
        let delta = incremental_samples
            .record(|| incremental_router.compile(&incremental_demand, &facts, 8));
        incremental_wire_ops += wire_ops(&delta);
    }
    let incremental = Metric::new(
        "internal_control",
        "router_incremental_full_recompile",
        topology.label(),
        incremental_started.elapsed(),
        incremental_samples,
    )
    .count("demand_atoms", demand.len() as u64)
    .count("wire_ops", incremental_wire_ops as u64)
    .note("legacy control: one whole-demand compile after each new atom; not the admission path");

    let mut admission_router = Router::new(RuleRegistry::default_widen_only());
    let started = Instant::now();
    let mut admission_samples = Samples::default();
    let admission_delta = admission_samples.record(|| admission_router.admit(&demand, &facts, 8));
    let admission = Metric::new(
        "internal_control",
        "router_pending_cohort_admit",
        topology.label(),
        started.elapsed(),
        admission_samples,
    )
    .count("demand_atoms", demand.len() as u64)
    .count("wire_ops", wire_ops(&admission_delta) as u64)
    .count(
        "wire_reqs",
        admission_router
            .plan()
            .reqs
            .values()
            .map(Vec::len)
            .sum::<usize>() as u64,
    )
    .note("one pending cohort routed and coalesced without incumbent rewrites");

    let started = Instant::now();
    let mut attach_samples = Samples::default();
    let mut attach_wire_ops = 0usize;
    for atom in &demand {
        let delta = attach_samples
            .record(|| admission_router.admit(&BTreeSet::from([atom.clone()]), &facts, 8));
        attach_wire_ops += wire_ops(&delta);
    }
    let attach = Metric::new(
        "internal_control",
        "router_active_attach",
        topology.label(),
        started.elapsed(),
        attach_samples,
    )
    .count("demand_atoms", demand.len() as u64)
    .count("wire_ops", attach_wire_ops as u64)
    .note("exact active coverage lookup only; zero compile and zero wire work");

    let mut active = demand.clone();
    let started = Instant::now();
    let mut withdraw_samples = Samples::default();
    let mut withdraw_wire_ops = 0usize;
    for atom in &demand {
        active.remove(atom);
        let delta = withdraw_samples.record(|| admission_router.withdraw(&active, 8));
        withdraw_wire_ops += wire_ops(&delta);
    }
    let withdraw = Metric::new(
        "internal_control",
        "router_incremental_withdraw",
        topology.label(),
        started.elapsed(),
        withdraw_samples,
    )
    .count("demand_atoms", demand.len() as u64)
    .count("wire_ops", withdraw_wire_ops as u64)
    .note("immutable router withdrawal only; excludes resolver snapshots and core diagnostics");

    let mut router = Router::new(RuleRegistry::default_widen_only());
    let started = Instant::now();
    let mut initial_samples = Samples::default();
    let initial_delta = initial_samples.record(|| router.compile(&demand, &facts, 8));
    let initial_elapsed = started.elapsed();
    let initial_ops = wire_ops(&initial_delta);
    let wire_reqs = router.plan().reqs.values().map(Vec::len).sum::<usize>();
    let initial = Metric::new(
        "internal_control",
        "router_initial_compile",
        topology.label(),
        initial_elapsed,
        initial_samples,
    )
    .count("demand_atoms", demand.len() as u64)
    .count("semantic_values", workload.semantic_values() as u64)
    .count("wire_reqs", wire_reqs as u64)
    .count("wire_ops", initial_ops as u64)
    .note("pure full router compile; all atoms target one .invalid relay partition");

    let started = Instant::now();
    let mut stable_samples = Samples::default();
    let mut stable_wire_ops = 0usize;
    for _ in 0..args.iterations {
        let delta = stable_samples.record(|| router.compile(&demand, &facts, 8));
        stable_wire_ops += wire_ops(&delta);
    }
    let stable = Metric::new(
        "internal_control",
        "router_stable_recompile",
        topology.label(),
        started.elapsed(),
        stable_samples,
    )
    .count("demand_atoms", demand.len() as u64)
    .count("wire_ops", stable_wire_ops as u64)
    .note("zero demand churn; includes routing, coalescing, diagnostics, and plan diff");

    let filters = demand
        .iter()
        .map(|atom| atom.filter.clone())
        .collect::<BTreeSet<_>>();
    let rules = RuleRegistry::default_widen_only();
    let started = Instant::now();
    let mut coalesce_samples = Samples::default();
    let mut survivors = 0usize;
    for _ in 0..args.iterations {
        survivors = coalesce_samples
            .record(|| rules.coalesce(filters.clone()))
            .len();
    }
    let coalesce = Metric::new(
        "internal_control",
        "selection_coalesce",
        topology.label(),
        started.elapsed(),
        coalesce_samples,
    )
    .count("input_filters", filters.len() as u64)
    .count("survivors", survivors as u64)
    .note("selection-only coalescer lower bound; Router owns per-relay/source partitioning");

    let plan = router.plan().clone();
    let started = Instant::now();
    let mut diff_samples = Samples::default();
    let mut diff_wire_ops = 0usize;
    for _ in 0..args.iterations {
        let delta = diff_samples.record(|| diff_plans(&plan, &plan));
        diff_wire_ops += wire_ops(&delta);
    }
    let diff = Metric::new(
        "internal_control",
        "wire_plan_diff_stable",
        topology.label(),
        started.elapsed(),
        diff_samples,
    )
    .count("wire_reqs", wire_reqs as u64)
    .count("wire_ops", diff_wire_ops as u64)
    .note("isolated byte-identical plan diff; evidence/status recompute is outside this seam");

    let started = Instant::now();
    let mut churn_samples = Samples::default();
    let mut churn_wire_ops = 0usize;
    for iteration in 0..args.iterations {
        let changed = changed_demand(&demand, iteration);
        let delta = churn_samples.record(|| router.compile(&changed, &facts, 8));
        churn_wire_ops += wire_ops(&delta);
        let delta = churn_samples.record(|| router.compile(&demand, &facts, 8));
        churn_wire_ops += wire_ops(&delta);
    }
    let churn = Metric::new(
        "internal_control",
        "router_one_atom_churn",
        topology.label(),
        started.elapsed(),
        churn_samples,
    )
    .count("demand_atoms", demand.len() as u64)
    .count("wire_ops", churn_wire_ops as u64)
    .note("one atom changes then returns to baseline per iteration");

    Ok(vec![
        incremental,
        admission,
        attach,
        withdraw,
        initial,
        stable,
        coalesce,
        diff,
        churn,
    ])
}

fn changed_demand(
    baseline: &BTreeSet<ContextualAtom>,
    iteration: usize,
) -> BTreeSet<ContextualAtom> {
    let mut changed = baseline.clone();
    if let Some(first) = baseline.first() {
        changed.remove(first);
        let mut replacement = first.clone();
        replacement.filter.since = Some(1_720_000_000 + iteration as u64);
        changed.insert(replacement);
    }
    changed
}

fn wire_ops(delta: &nmp_router::WireDelta) -> usize {
    delta.ops.iter().map(|(_, ops)| ops.len()).sum()
}
