use nmp::mechanism::core::EngineCore;
use nmp_store::RedbStore;

use crate::measure::Metric;

pub(crate) fn reset(core: &mut EngineCore<RedbStore>) {
    core.bench_reset_query_work();
    core.bench_reset_lifecycle_work();
    core.bench_reset_coverage_reads();
    core.bench_reset_admission_work();
    core.bench_reset_withdrawal_work();
}

pub(crate) fn apply_core_work(core: &EngineCore<RedbStore>, metric: Metric) -> Metric {
    let (index_rows, event_values, examined_rows) = core.bench_query_work();
    let (projection_reads, router_compiles, history_reads) = core.bench_lifecycle_work();
    let admission = core.bench_admission_work();
    let withdrawal = core.bench_withdrawal_work();
    let census = core.bench_ownership_census();
    let metric = metric
        .count("projection_reads", projection_reads)
        .count("router_compiles", router_compiles)
        .count("history_projection_reads", history_reads)
        .count("index_rows", index_rows)
        .count("event_values", event_values)
        .count("examined_rows", examined_rows)
        .count("coverage_reads", core.bench_coverage_reads())
        .count(
            "admission_pending_atoms_rebuilt",
            admission.pending_atoms_rebuilt,
        )
        .count(
            "pending_cohort_atoms_reconciled",
            admission.pending_cohort_atoms_reconciled,
        )
        .count(
            "attribution_atoms_rebuilt",
            admission.attribution_atoms_rebuilt,
        )
        .count(
            "admission_evidence_candidates_examined",
            admission.evidence_candidates_examined,
        )
        .count(
            "request_target_demand_keys_touched",
            admission.request_target_demand_keys_touched,
        )
        .count(
            "request_target_candidates_examined",
            admission.request_target_candidates_examined,
        )
        .count(
            "request_claim_entries_examined",
            admission.request_claim_entries_examined,
        )
        .count(
            "request_owner_entries_examined",
            admission.request_owner_entries_examined,
        )
        .count(
            "request_claim_transfer_attempts",
            admission.request_claim_transfer_attempts,
        )
        .count(
            "request_claim_transfer_claims_attempted",
            admission.request_claim_transfer_claims_attempted,
        )
        .count(
            "request_claim_transfer_commits",
            admission.request_claim_transfer_commits,
        )
        .count(
            "request_claim_transfer_failures",
            admission.request_claim_transfer_failures,
        )
        .count(
            "admission_diagnostic_snapshots_built",
            admission.diagnostic_snapshots_built,
        )
        .count("router_cohort_compiles", admission.cohort_compiles)
        .count(
            "router_incumbent_active_entries_visited",
            admission.incumbent_active_entries_visited,
        )
        .count(
            "router_incumbent_plan_requests_visited",
            admission.incumbent_plan_requests_visited,
        )
        .count(
            "router_incumbent_limited_entries_visited",
            admission.incumbent_limited_entries_visited,
        )
        .count(
            "router_incumbent_refusal_entries_visited",
            admission.incumbent_refusal_entries_visited,
        )
        .count(
            "router_active_entries_appended",
            admission.active_entries_appended,
        )
        .count(
            "router_request_edges_appended",
            admission.request_edges_appended,
        )
        .count(
            "router_metadata_entries_examined",
            admission.metadata_entries_examined,
        )
        .count("handles_detached", withdrawal.handles_detached)
        .count("resolver_delta_ops", withdrawal.resolver_delta_ops_consumed)
        .count(
            "resolver_owner_keys_touched",
            withdrawal.resolver_owner_keys_touched,
        )
        .count(
            "resolver_surviving_atoms_examined",
            withdrawal.resolver_surviving_atoms_examined,
        )
        .count("pending_atoms_rebuilt", withdrawal.pending_atoms_rebuilt)
        .count(
            "evidence_candidates_examined",
            withdrawal.evidence_candidates_examined,
        )
        .count(
            "routing_evidence_owner_keys_touched",
            withdrawal.routing_evidence_owner_keys_touched,
        )
        .count(
            "diagnostic_snapshots_built",
            withdrawal.diagnostic_snapshots_built,
        )
        .count("exact_atoms_closed", withdrawal.exact_atoms_closed)
        .count("request_edges", withdrawal.request_edges_touched)
        .count(
            "plan_request_entries_visited",
            withdrawal.plan_request_entries_visited,
        )
        .count("requests_closed", withdrawal.requests_closed)
        .count(
            "coverage_edges_released",
            withdrawal.physical_coverage_edges_released,
        )
        .count("diagnostic_refreshes", withdrawal.diagnostic_refreshes)
        .count(
            "diagnostic_requests_visited",
            withdrawal.diagnostic_requests_visited,
        )
        .count(
            "nip77_plan_children_touched",
            withdrawal.nip77_plan_children_touched,
        );
    crate::ownership::apply(metric, census)
}
