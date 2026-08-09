use anyhow::{ensure, Context, Result};

use crate::args::{DemandShape, LifecycleSchedule};
use crate::measure::Metric;

pub(crate) fn validate_interleaved(
    churn: &Metric,
    flush: &Metric,
    observations: usize,
) -> Result<()> {
    ensure!(
        count(churn, "projection_reads") == observations as u64
            && count(churn, "router_compiles") == 0
            && count(churn, "wire_ops") == 0
            && count(churn, "handles_detached") == observations as u64
            && count(churn, "pending_atoms_rebuilt") == 0,
        "interleaved lifecycle must remain exact local work with no pending census"
    );
    ensure!(count(flush, "effects") == 0);
    ensure_zero_census(churn)?;
    Ok(())
}

pub(crate) fn validate_lifecycle(
    open: &Metric,
    admission: &Metric,
    handoff: Option<&Metric>,
    close: &Metric,
    shape: DemandShape,
    schedule: LifecycleSchedule,
    observations: usize,
) -> Result<()> {
    let observations = observations as u64;
    ensure!(
        count(open, "projection_reads") == observations
            && count(open, "unique_observation_ids") == observations
            && count(open, "row_frames") == observations
            && count(open, "evidence_frames") == observations
            && count(open, "router_compiles") == 0
            && count(open, "wire_ops") == 0,
        "every open must keep an independent local projection and leave wire admission pending"
    );
    if shape == DemandShape::ProfileAuthors {
        ensure!(
            count(open, "row_deltas") == observations,
            "every avatar observation must receive its own cached old profile immediately"
        );
    }
    ensure!(
        count(close, "projection_reads") == 0
            && count(close, "coverage_reads") == 0
            && count(close, "index_rows") == 0
            && count(close, "event_values") == 0
            && count(close, "evidence_candidates_examined") == 0
            && count(close, "diagnostic_snapshots_built") == 0
            && count(close, "router_compiles") == 0
            && count(close, "wire_reqs") == 0
            && count(close, "handles_detached") == observations
            && count(close, "pending_atoms_rebuilt") == 0,
        "ordinary cancellation must be exact delta work with no store reads or pending census (coverage_reads={})",
        count(close, "coverage_reads")
    );
    let expected_atom_closes = match shape {
        DemandShape::ExactDuplicate => 1,
        DemandShape::CompatibleDistinct
        | DemandShape::ProfileAuthors
        | DemandShape::LimitedIncompatible
        | DemandShape::UnlimitedMultiAxisIncompatible => observations,
        DemandShape::All => unreachable!("matrix expands the all selector"),
    };
    ensure!(count(close, "exact_atoms_closed") == expected_atom_closes);
    if schedule == LifecycleSchedule::BeforeAdmission {
        ensure!(handoff.is_none());
        validate_pre_admission_cancel(admission, close)?;
    } else {
        validate_admitted(
            open,
            admission,
            handoff.context("admitted lifecycle omitted local handoff phase")?,
            close,
            shape,
            observations,
        )?;
    }
    ensure_zero_census(close)?;
    Ok(())
}

pub(crate) fn ensure_zero_census(metric: &Metric) -> Result<()> {
    crate::ownership::ensure_zero(metric)
}

fn validate_pre_admission_cancel(admission: &Metric, close: &Metric) -> Result<()> {
    ensure!(
        count(admission, "effects") == 0
            && count(admission, "router_compiles") == 0
            && count(close, "wire_ops") == 0
            && count(close, "request_edges") == 0,
        "cancelling a pending cohort must leave nothing to admit and touch no request edge"
    );
    Ok(())
}

fn validate_admitted(
    open: &Metric,
    admission: &Metric,
    handoff: &Metric,
    close: &Metric,
    shape: DemandShape,
    observations: u64,
) -> Result<()> {
    let logical_demands = if shape == DemandShape::ExactDuplicate {
        1
    } else {
        observations
    };
    ensure!(
        count(admission, "router_compiles") == 1
            && count(admission, "projection_reads") == 0
            && count(admission, "wire_reqs") > 0
            && count(admission, "wire_closes") == 0,
        "one pending cohort must compile once without reprojecting or closing incumbent work"
    );
    ensure!(
        count(admission, "pending_cohort_atoms_reconciled") == logical_demands
            && count(admission, "router_cohort_compiles") == 1
            && count(admission, "router_incumbent_active_entries_visited") == 0
            && count(admission, "router_incumbent_plan_requests_visited") == 0
            && count(admission, "router_incumbent_limited_entries_visited") == 0
            && count(admission, "router_incumbent_refusal_entries_visited") == 0
            && count(admission, "router_active_entries_appended") == 0
            && count(admission, "router_request_edges_appended") == logical_demands
            && count(admission, "request_target_demand_keys_touched") == 0
            && count(admission, "request_target_candidates_examined") == 0,
        "admission must compile only the submitted logical cohort before local acceptance (logical={}, reconciled={}, compiles={}, active={}, edges={})",
        logical_demands,
        count(admission, "pending_cohort_atoms_reconciled"),
        count(admission, "router_cohort_compiles"),
        count(admission, "router_active_entries_appended"),
        count(admission, "router_request_edges_appended"),
    );
    ensure!(
        count(handoff, "request_target_demand_keys_touched") == logical_demands
            && count(handoff, "request_target_candidates_examined") == observations,
        "accepted requests must examine only their exact app targets (logical={}, target_keys={}, candidates={}, observations={})",
        logical_demands,
        count(handoff, "request_target_demand_keys_touched"),
        count(handoff, "request_target_candidates_examined"),
        observations,
    );
    ensure!(
        count(close, "wire_closes") == count(admission, "wire_reqs"),
        "closing final owners must retire exactly their admitted requests"
    );
    match shape {
        DemandShape::ExactDuplicate => ensure!(
            count(admission, "wire_tag_occurrences") == 1
                && count(admission, "wire_unique_tag_values") == 1
                && count(admission, "wire_reqs_with_limit") == 0,
            "duplicate demand must produce one exact unlimited wire value"
        ),
        DemandShape::CompatibleDistinct => ensure!(
            count(admission, "wire_tag_occurrences") == observations
                && count(admission, "wire_unique_tag_values") == observations
                && count(admission, "wire_reqs_with_limit") == 0,
            "compatible tag demand must cover every value exactly once without a limit"
        ),
        DemandShape::ProfileAuthors => ensure!(
            count(open, "row_deltas") == observations
                && count(admission, "wire_author_occurrences") == observations
                && count(admission, "wire_unique_authors") == observations
                && count(admission, "wire_reqs_with_limit") == 0,
            "avatar demand must seed and cover every author exactly once without a limit"
        ),
        DemandShape::LimitedIncompatible => ensure!(
            count(admission, "wire_reqs") == observations
                && count(admission, "wire_tag_occurrences") == observations
                && count(admission, "wire_unique_tag_values") == observations
                && count(admission, "wire_reqs_with_limit") == observations,
            "limited-incompatible demand must remain one limit:1 request per observation"
        ),
        DemandShape::UnlimitedMultiAxisIncompatible => ensure!(
            count(admission, "wire_reqs") == observations
                && count(admission, "wire_tag_occurrences") == observations
                && count(admission, "wire_unique_tag_values") == observations
                && count(admission, "wire_reqs_with_limit") == 0,
            "multi-axis-incompatible demand must remain one exact unlimited request per observation"
        ),
        DemandShape::All => unreachable!("matrix expands the all selector"),
    }
    Ok(())
}

fn count(metric: &Metric, key: &str) -> u64 {
    metric.counts.get(key).copied().unwrap_or_default()
}
