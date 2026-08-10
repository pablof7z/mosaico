use anyhow::{ensure, Context, Result};
use nmp::mechanism::core::CoreOwnershipCensus;

use crate::measure::Metric;

// The struct pattern deliberately has no `..`: an NMP census addition breaks
// this harness until the new owner is named, reported, and included in the
// zero-teardown oracle. Missing metric keys also fail instead of reading as 0.
macro_rules! define_census_mapping {
    ($($field:ident => $key:literal),+ $(,)?) => {
        pub(crate) const CENSUS_KEYS: &[&str] = &[$($key),+];

        pub(crate) fn apply(metric: Metric, census: CoreOwnershipCensus) -> Metric {
            let CoreOwnershipCensus { $($field),+ } = census;
            let mut metric = metric;
            $(metric = metric.count($key, $field as u64);)+
            metric
        }
    };
}

define_census_mapping! {
    observations => "active_observations",
    branch_handles => "active_branch_handles",
    retained_freshness_source_edges => "retained_freshness_source_edges",
    request_target_handles => "request_target_handles",
    request_target_demand_keys => "request_target_demand_keys",
    request_target_edges => "request_target_edges",
    request_target_refs => "request_target_refs",
    active_request_target_handles => "active_request_target_handles",
    active_request_target_demand_keys => "active_request_target_demand_keys",
    active_request_target_edges => "active_request_target_edges",
    active_request_target_refs => "active_request_target_refs",
    history_sessions => "active_history_sessions",
    history_handles => "active_history_handles",
    resolver_active_atoms => "active_resolver_atoms",
    pending_wire_atoms => "pending_wire_atoms",
    pending_resolver_wire_closes => "pending_resolver_wire_closes",
    wire_handles => "active_wire_handles",
    wire_handle_demand_ref_handles => "wire_handle_demand_ref_handles",
    wire_handle_demand_ref_keys => "wire_handle_demand_ref_keys",
    wire_handle_demand_refs => "wire_handle_demand_refs",
    wire_handle_coverage_ref_handles => "wire_handle_coverage_ref_handles",
    wire_handle_coverage_ref_keys => "wire_handle_coverage_ref_keys",
    wire_handle_coverage_refs => "wire_handle_coverage_refs",
    wire_owner_keys => "wire_owner_keys",
    wire_reverse_owner_keys => "wire_reverse_owner_keys",
    wire_coverage_keys => "wire_coverage_keys",
    wire_coverage_edges => "wire_coverage_edges",
    wire_demand_keys => "wire_demand_keys",
    wire_demand_edges => "wire_demand_edges",
    wire_routing_evidence_keys => "wire_routing_evidence_keys",
    wire_routing_evidence_facts => "wire_routing_evidence_facts",
    wire_routing_evidence_refs => "wire_routing_evidence_refs",
    active_physical_requests => "active_physical_requests",
    pending_execution_owner_keys => "pending_execution_owner_keys",
    pending_execution_owners => "pending_execution_owners",
    request_attempts => "request_attempts",
    request_attempt_sub_keys => "request_attempt_sub_keys",
    request_attempt_sub_edges => "request_attempt_sub_edges",
    request_attempt_session_keys => "request_attempt_session_keys",
    request_attempt_session_edges => "request_attempt_session_edges",
    request_retry_jobs => "request_retry_jobs",
    request_retry_sub_keys => "request_retry_sub_keys",
    request_retry_session_keys => "request_retry_session_keys",
    request_retry_session_edges => "request_retry_session_edges",
    request_replacement_jobs => "request_replacement_jobs",
    request_replacement_session_keys => "request_replacement_session_keys",
    request_replacement_session_edges => "request_replacement_session_edges",
    active_execution_owners => "active_execution_owners",
    active_execution_owner_keys => "active_execution_owner_keys",
    live_wire_owners => "live_wire_owners",
    pending_request_claim_transfer_jobs => "pending_request_claim_transfer_jobs",
    pending_request_claim_transfer_claims => "pending_request_claim_transfer_claims",
    attribution_inflight_subs => "attribution_inflight_subs",
    attribution_wire_keys => "attribution_wire_keys",
    attribution_shape_keys => "attribution_shape_keys",
    attribution_active_demands => "attribution_active_demands",
    attribution_active_shape_keys => "attribution_active_shape_keys",
    attribution_active_shape_refs => "attribution_active_shape_refs",
    attribution_live_request_keys => "attribution_live_request_keys",
    attribution_live_shape_keys => "attribution_live_shape_keys",
    attribution_live_shape_refs => "attribution_live_shape_refs",
    attribution_inflight_shape_keys => "attribution_inflight_shape_keys",
    attribution_inflight_shape_refs => "attribution_inflight_shape_refs",
    projected_rejection_demand_keys => "projected_rejection_demand_keys",
    projected_rejection_owner_keys => "projected_rejection_owner_keys",
    projected_rejection_owner_refs => "projected_rejection_owner_refs",
    planned_read_sessions => "planned_read_sessions",
    planned_read_relays => "planned_read_relays",
    plan_execution_metadata => "plan_execution_metadata",
    plan_execution_claims => "plan_execution_claims",
    plan_execution_owner_demands => "plan_execution_owner_demands",
    active_nip77_live => "active_nip77_live",
    pending_neg_handoffs => "pending_neg_handoffs",
    pending_neg_plan_keys => "pending_neg_plan_keys",
    pending_neg_plan_edges => "pending_neg_plan_edges",
    neg_sessions => "neg_sessions",
    neg_session_plan_keys => "neg_session_plan_keys",
    neg_session_plan_edges => "neg_session_plan_edges",
    pending_backfills => "pending_backfills",
    pending_backfill_plan_keys => "pending_backfill_plan_keys",
    pending_backfill_plan_edges => "pending_backfill_plan_edges",
    router_active_demands => "router_active_demands",
    router_request_demand_keys => "router_request_demand_keys",
    router_request_demand_edges => "router_request_demand_edges",
    router_active_requests => "router_active_requests",
    router_request_coverage_keys => "router_request_coverage_keys",
    router_request_position_keys => "router_request_position_keys",
    router_request_exact_filter_keys => "router_request_exact_filter_keys",
    router_physical_request_claim_keys => "router_physical_request_claim_keys",
    router_physical_claim_keys => "router_physical_claim_keys",
    router_physical_claim_edges => "router_physical_claim_edges",
    router_physical_request_contribution_keys => "router_physical_request_contribution_keys",
    router_physical_demand_keys => "router_physical_demand_keys",
    router_physical_demand_edges => "router_physical_demand_edges",
    router_request_owner_contribution_keys => "router_request_owner_contribution_keys",
    router_request_claim_owner_count_keys => "router_request_claim_owner_count_keys",
    router_request_provenance_owner_count_keys => "router_request_provenance_owner_count_keys",
    router_request_demand_coverage_owner_count_keys => "router_request_demand_coverage_owner_count_keys",
    router_coverage_assignment_keys => "router_coverage_assignment_keys",
    router_coverage_assignment_edges => "router_coverage_assignment_edges",
    router_refused_coverage_assignment_demands => "router_refused_coverage_assignment_demands",
    router_refused_coverage_assignment_authors => "router_refused_coverage_assignment_authors",
    router_active_outbox_authors => "router_active_outbox_authors",
    router_refusal_demand_keys => "router_refusal_demand_keys",
    router_refusal_demand_edges => "router_refusal_demand_edges",
    router_refused_request_owner_keys => "router_refused_request_owner_keys",
    router_refused_session_owner_keys => "router_refused_session_owner_keys",
    router_diagnostic_author_session_keys => "router_diagnostic_author_session_keys",
    router_diagnostic_author_edges => "router_diagnostic_author_edges",
    router_uncovered_demand_keys => "router_uncovered_demand_keys",
    router_uncovered_author_keys => "router_uncovered_author_keys",
    router_uncovered_author_refs => "router_uncovered_author_refs",
    router_plan_sessions => "router_plan_sessions",
    router_plan_limited_demands => "router_plan_limited_demands",
    router_plan_refused_sessions => "router_plan_refused_sessions",
    router_plan_subscription_shortfalls => "router_plan_subscription_shortfalls",
    router_diagnostic_sessions => "router_diagnostic_sessions",
    router_diagnostic_uncovered_authors => "router_diagnostic_uncovered_authors",
    router_diagnostic_sessions_refused_by_cap => "router_diagnostic_sessions_refused_by_cap",
    router_diagnostic_sessions_refused_by_subscription_budget => "router_diagnostic_sessions_refused_by_subscription_budget",
    router_diagnostic_dropped_merge_rules => "router_diagnostic_dropped_merge_rules",
}

pub(crate) fn ensure_zero(metric: &Metric) -> Result<()> {
    for key in CENSUS_KEYS {
        let value = metric
            .counts
            .get(key)
            .with_context(|| format!("teardown metric omitted ownership key {key}"))?;
        ensure!(*value == 0, "teardown leaked {key}={value}");
    }
    Ok(())
}
