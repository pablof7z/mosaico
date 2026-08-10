use nmp_router::{AdmissionWork, WithdrawalWork};

use crate::measure::Metric;

pub(crate) fn admission(metric: Metric, work: AdmissionWork) -> Metric {
    let AdmissionWork {
        cohort_compiles,
        incumbent_active_entries_visited,
        incumbent_plan_requests_visited,
        incumbent_limited_entries_visited,
        incumbent_refusal_entries_visited,
        active_entries_appended,
        request_edges_appended,
        metadata_entries_examined,
    } = work;
    metric
        .count("cohort_compiles", cohort_compiles)
        .count(
            "incumbent_active_entries_visited",
            incumbent_active_entries_visited,
        )
        .count(
            "incumbent_plan_requests_visited",
            incumbent_plan_requests_visited,
        )
        .count(
            "incumbent_limited_entries_visited",
            incumbent_limited_entries_visited,
        )
        .count(
            "incumbent_refusal_entries_visited",
            incumbent_refusal_entries_visited,
        )
        .count("active_entries_appended", active_entries_appended)
        .count("request_edges_appended", request_edges_appended)
        .count("metadata_entries_examined", metadata_entries_examined)
}

pub(crate) fn withdrawal(metric: Metric, work: WithdrawalWork) -> Metric {
    let WithdrawalWork {
        dropped_atoms,
        request_edges_touched,
        metadata_owner_entries_touched,
        metadata_claim_entries_touched,
        metadata_assignment_entries_touched,
        metadata_provenance_entries_touched,
        plan_request_entries_visited,
        requests_closed,
        physical_coverage_edges_released,
        diagnostic_rebuilds,
        diagnostic_requests_visited,
    } = work;
    metric
        .count("dropped_atoms", dropped_atoms)
        .count("request_edges", request_edges_touched)
        .count(
            "metadata_owner_entries_touched",
            metadata_owner_entries_touched,
        )
        .count(
            "metadata_claim_entries_touched",
            metadata_claim_entries_touched,
        )
        .count(
            "metadata_assignment_entries_touched",
            metadata_assignment_entries_touched,
        )
        .count(
            "metadata_provenance_entries_touched",
            metadata_provenance_entries_touched,
        )
        .count("plan_request_entries_visited", plan_request_entries_visited)
        .count("requests_closed", requests_closed)
        .count("coverage_edges_released", physical_coverage_edges_released)
        .count("diagnostic_refreshes", diagnostic_rebuilds)
        .count("diagnostic_requests_visited", diagnostic_requests_visited)
}
