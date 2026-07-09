//! `probe validate`: one honest validation envelope over the existing Trellis
//! probes. It gathers oracle/seam/resource checks for every request, then adds
//! target-specific explanation, preview, acid, and replay evidence when the
//! caller supplies a handle, fact, or replay capsule.

use self::report::{
    bool_at, oracle_summary, push_check, seams_status, seams_summary, str_at, verdict,
};
use super::{oracle, seams, DaemonState};
use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

mod alias;
mod awareness;
mod channel;
mod commit;
mod coverage;
mod cursor;
mod durable_outbox;
mod envelope;
mod event;
mod explain_evidence;
mod hook_context;
mod identity;
mod inbox;
mod input;
mod joined;
mod label;
mod llm;
mod lookup;
mod membership;
mod message;
mod outbox;
mod params;
mod quarantine;
mod readiness_attempt;
mod receipt;
mod recipient;
mod report;
mod resource_path;
mod resource_state;
mod runs;
mod scope;
mod session;
mod session_consistency;
mod session_start;
mod session_watch;
mod state_check;
mod status;
mod subscription;
mod table_samples;
mod target;
mod target_checks;
mod turn;
mod txn;
mod workspace;

pub(super) fn validate_value(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let input = input::ValidationInput::collect(state, params);

    let why = explain_evidence::why(state, input.handle);
    let (explanation, explain_error) = explain_evidence::explanation(state, &input.explain_handle);

    let surface = scope::selected_surface(
        input.target,
        input.explain_handle.as_ref(),
        &input.target_checks,
        explanation.as_ref(),
        input.cause_label_evidence.as_ref(),
        input.fact_surface,
        input.capsule_surface,
    );
    let global_validation = input.raw_target.filter(|target| *target != "all").is_none()
        && input.parameter_evidence.is_empty()
        && input.target_evidence.is_none()
        && input.fact_evidence.is_none()
        && input.capsule.is_none()
        && surface.is_none();

    let mut checks = Vec::new();
    let mut limitations = Vec::new();

    if !input.parameter_evidence.is_empty() {
        let names = input
            .parameter_evidence
            .iter()
            .filter_map(|v| v.get("parameter").and_then(Value::as_str))
            .collect::<Vec<_>>();
        push_check(
            &mut checks,
            "input",
            "failed",
            format!("invalid validate parameter(s): {}", names.join(", ")),
        );
        limitations.extend(
            input
                .parameter_evidence
                .iter()
                .filter_map(|v| v.get("reason").and_then(Value::as_str))
                .map(str::to_string),
        );
    }

    let oracle = oracle::oracle_value(state);
    push_check(
        &mut checks,
        "oracle",
        if bool_at(&oracle, "ok") {
            "passed"
        } else {
            "failed"
        },
        oracle_summary(&oracle),
    );

    let seams = seams::seams_value();
    let global_seams_checked = global_validation || input.target_checks.global_seams_checked();
    if surface.is_some() || global_seams_checked {
        let seam_status = seams_status(&seams, surface.as_deref());
        push_check(
            &mut checks,
            "seams",
            seam_status,
            seams_summary(&seams, surface.as_deref()),
        );
        if seam_status == "not_proven" {
            if !bool_at(&oracle, "surface_correctness_proven") {
                limitations.push(
                    "oracle proves graph bookkeeping, not host-effect correctness".to_string(),
                );
            }
            limitations.push("some host-effect paths are not covered by Trellis seams".to_string());
        }
    }

    if let Some(v) = &input.target_evidence {
        let status = if v.get("valid").and_then(Value::as_bool) == Some(false) {
            "failed"
        } else {
            "not_proven"
        };
        push_check(
            &mut checks,
            "target",
            status,
            str_at(v, "summary").to_string(),
        );
        limitations.push(str_at(v, "reason").to_string());
    }

    if let Some(v) = &input.cause_label_evidence {
        push_check(
            &mut checks,
            "cause_label",
            "passed",
            str_at(v, "summary").to_string(),
        );
    }
    input
        .target_checks
        .push_checks(&mut checks, &mut limitations);

    if let Some(v) = &why {
        durable_outbox::push_why_check(&mut checks, &mut limitations, v, &input.target_checks);
    }

    explain_evidence::push_checks(
        &mut checks,
        &mut limitations,
        explanation.as_ref(),
        explain_error.as_deref(),
        &input.target_checks,
    );

    if let Some(v) = &input.fact_evidence {
        if bool_at(v, "supported") {
            push_check(
                &mut checks,
                "fact",
                "passed",
                str_at(v, "summary").to_string(),
            );
        } else if v.get("valid").and_then(Value::as_bool) == Some(false) {
            push_check(
                &mut checks,
                "fact",
                "failed",
                str_at(v, "summary").to_string(),
            );
            limitations.push(str_at(v, "reason").to_string());
        } else {
            push_check(
                &mut checks,
                "fact",
                "not_proven",
                str_at(v, "summary").to_string(),
            );
            limitations.push(str_at(v, "reason").to_string());
        }
    }

    let state_evidence = resource_state::collect(
        state,
        params,
        surface.as_deref(),
        global_seams_checked,
        input.has_malformed_target,
        &input.target_checks,
        input.handle,
        why.as_ref(),
        global_validation,
        &mut checks,
        &mut limitations,
    );
    let active_runs = runs::collect(
        state,
        params,
        surface.as_deref(),
        input.fact_surface,
        input.handle,
        why.as_ref(),
        input.capsule,
        &mut checks,
        &mut limitations,
    );

    let verdict = verdict(&checks, &limitations);
    Ok(envelope::build(
        input.raw_target,
        surface,
        input.handle,
        input.target.filter(|_| input.explain_handle.is_some()),
        input.capsule,
        verdict,
        checks,
        limitations,
        oracle,
        seams,
        state_evidence.stats,
        state_evidence.stats_error,
        state_evidence.surface_state,
        state_evidence.state_evidence,
        state_evidence.session_consistency,
        state_evidence.surface_states,
        state_evidence.state_error,
        why,
        explanation,
        explain_error,
        input.parameter_evidence,
        input.target_evidence,
        input.cause_label_evidence,
        input.target_checks,
        input.fact_evidence,
        active_runs.simulation,
        active_runs.simulate_error,
        active_runs.acid,
        active_runs.acid_error,
        active_runs.replay,
        active_runs.replay_error,
    ))
}
