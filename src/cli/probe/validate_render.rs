//! Human render for `probe validate`: verdict first, then the named checks and
//! explicit limitations. Full nested oracle/seam/why/explain/simulate/replay
//! evidence remains available with `--json`.

use serde_json::Value;
use std::fmt::Write as _;

mod alias_tail;
mod commit_tail;
mod coverage_tail;
mod cursor_tail;
mod error_tail;
mod event_tail;
mod fabric_tail;
mod hook_context_tail;
mod identity_tail;
mod inbox_tail;
mod input_tail;
mod joined_tail;
mod llm_tail;
mod membership_tail;
mod outbox_tail;
mod quarantine_tail;
mod readiness_attempt_tail;
mod receipt_tail;
mod recipient_tail;
mod session_consistency_tail;
mod session_lifecycle_tail;
mod session_tail;
mod state_tail;
mod status_tail;
mod subscription_tail;
mod turn_tail;
mod txn_tail;
mod workspace_tail;

pub(in crate::cli) fn render_validate(v: &Value) -> String {
    let mut out = String::new();
    let target = v
        .get("target")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("all");
    let _ = writeln!(
        out,
        "validate {target}  verdict={}  ok={}",
        str_at(v, "verdict"),
        bool_at(v, "ok")
    );

    let empty = Vec::new();
    for check in v.get("checks").and_then(Value::as_array).unwrap_or(&empty) {
        let _ = writeln!(
            out,
            "  {:<19} {:<10} {}",
            str_at(check, "name"),
            str_at(check, "status"),
            str_at(check, "summary")
        );
    }

    let limitations = v
        .get("limitations")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    if !limitations.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "limitations");
        for limitation in limitations.iter().filter_map(Value::as_str) {
            let _ = writeln!(out, "  - {limitation}");
        }
    }

    error_tail::render(&mut out, v);
    if let Some(params) = v.get("parameter_evidence").and_then(Value::as_array) {
        input_tail::render_parameters(&mut out, params);
    }
    if let Some(target) = v.get("target_evidence").filter(|v| !v.is_null()) {
        input_tail::render_target(&mut out, target);
    }
    if let Some(state) = v.get("state_evidence").filter(|v| !v.is_null()) {
        state_tail::render(&mut out, state);
    } else if let Some(state) = v.get("state").filter(|v| {
        v.is_object()
            && v.get("check_status")
                .and_then(Value::as_str)
                .is_some_and(|status| !status.is_empty())
    }) {
        state_tail::render_surface_states(&mut out, std::slice::from_ref(state));
    }
    if let Some(consistency) = v.get("session_consistency").filter(|v| !v.is_null()) {
        session_consistency_tail::render(&mut out, consistency);
    }
    if let Some(states) = v.get("surface_states").and_then(Value::as_array) {
        state_tail::render_surface_states(&mut out, states);
    }
    if let Some(cause) = v.get("cause_label_evidence").filter(|v| !v.is_null()) {
        input_tail::render_cause_label(&mut out, cause);
    }
    if let Some(channel) = v.get("channel_evidence").filter(|v| !v.is_null()) {
        fabric_tail::render_channel(&mut out, channel);
    }
    if let Some(commit) = v.get("commit_evidence").filter(|v| !v.is_null()) {
        commit_tail::render(&mut out, commit);
    }
    if let Some(coverage) = v.get("coverage_evidence").filter(|v| !v.is_null()) {
        coverage_tail::render(&mut out, coverage);
    }
    if let Some(alias) = v.get("alias_evidence").filter(|v| !v.is_null()) {
        alias_tail::render(&mut out, alias);
    }
    if let Some(workspace) = v.get("workspace_evidence").filter(|v| !v.is_null()) {
        workspace_tail::render(&mut out, workspace);
    }
    if let Some(membership) = v.get("membership_evidence").filter(|v| !v.is_null()) {
        membership_tail::render(&mut out, membership);
    }
    if let Some(awareness) = v.get("awareness_evidence").filter(|v| !v.is_null()) {
        fabric_tail::render_awareness(&mut out, awareness);
    }
    if let Some(event) = v.get("event_evidence").filter(|v| !v.is_null()) {
        event_tail::render(&mut out, event);
    }
    if let Some(inbox) = v.get("inbox_evidence").filter(|v| !v.is_null()) {
        inbox_tail::render(&mut out, inbox);
    }
    if let Some(joined) = v.get("joined_evidence").filter(|v| !v.is_null()) {
        joined_tail::render(&mut out, joined);
    }
    if let Some(quarantine) = v.get("quarantine_evidence").filter(|v| !v.is_null()) {
        quarantine_tail::render(&mut out, quarantine);
    }
    if let Some(message) = v.get("message_evidence").filter(|v| !v.is_null()) {
        fabric_tail::render_message(&mut out, message);
    }
    if let Some(recipient) = v.get("recipient_evidence").filter(|v| !v.is_null()) {
        recipient_tail::render(&mut out, recipient);
    }
    if let Some(snapshot) = v
        .get("membership_snapshot_evidence")
        .filter(|v| !v.is_null())
    {
        membership_tail::render_snapshot(&mut out, snapshot);
    }
    if let Some(attempt) = v.get("readiness_attempt_evidence").filter(|v| !v.is_null()) {
        readiness_attempt_tail::render(&mut out, attempt);
    }
    if let Some(identity) = v.get("identity_evidence").filter(|v| !v.is_null()) {
        identity_tail::render(&mut out, identity);
    }
    if let Some(hook) = v.get("hook_context_evidence").filter(|v| !v.is_null()) {
        hook_context_tail::render(&mut out, hook);
    }
    if let Some(llm) = v.get("llm_evidence").filter(|v| !v.is_null()) {
        llm_tail::render(&mut out, llm);
    }
    if let Some(txn) = v.get("txn_evidence").filter(|v| !v.is_null()) {
        txn_tail::render(&mut out, txn);
    }
    if let Some(receipt) = v.get("receipt_evidence").filter(|v| !v.is_null()) {
        receipt_tail::render(&mut out, receipt);
    }
    if let Some(subscription) = v.get("subscription_evidence").filter(|v| !v.is_null()) {
        subscription_tail::render(&mut out, subscription);
    }
    if let Some(turn) = v.get("turn_evidence").filter(|v| !v.is_null()) {
        turn_tail::render(&mut out, turn);
    }
    if let Some(cursor) = v.get("cursor_evidence").filter(|v| !v.is_null()) {
        cursor_tail::render(&mut out, cursor);
    }
    if let Some(session) = v.get("session_evidence").filter(|v| !v.is_null()) {
        session_tail::render(&mut out, session);
    }
    if let Some(status) = v.get("status_evidence").filter(|v| !v.is_null()) {
        status_tail::render(&mut out, status);
    }
    if let Some(outbox) = v.get("outbox_evidence").filter(|v| !v.is_null()) {
        outbox_tail::render(&mut out, outbox);
    }
    if let Some(session_start) = v.get("session_start_evidence").filter(|v| !v.is_null()) {
        session_lifecycle_tail::render_session_start(&mut out, session_start);
    }
    if let Some(session_watch) = v.get("session_watch_evidence").filter(|v| !v.is_null()) {
        session_lifecycle_tail::render_session_watch(&mut out, session_watch);
    }
    if let Some(fact) = v.get("fact_evidence").filter(|v| !v.is_null()) {
        input_tail::render_fact(&mut out, fact);
    }
    if let Some(why) = v.get("why").filter(|v| !v.is_null()) {
        append_block(&mut out, super::render::render_why(why));
    }
    if let Some(sim) = v.get("simulate").filter(|v| !v.is_null()) {
        append_block(&mut out, super::render::render_simulate(sim));
    }
    if let Some(acid) = v.get("acid").filter(|v| !v.is_null()) {
        append_block(&mut out, super::render::render_acid(acid));
    }
    if let Some(explain) = v.get("explain").filter(|v| !v.is_null()) {
        render_explain_tail(&mut out, explain);
    }
    if let Some(replay) = v.get("replay").filter(|v| !v.is_null()) {
        append_block(&mut out, super::render::render_replay(replay));
    }
    out
}

fn append_block(out: &mut String, block: String) {
    if block.trim().is_empty() {
        return;
    }
    let _ = writeln!(out);
    out.push_str(&block);
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

fn render_explain_tail(out: &mut String, explain: &Value) {
    let empty = Vec::new();
    let receipts = explain
        .get("receipts")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    if receipts.is_empty() && explain.get("llm_call").is_none_or(Value::is_null) {
        return;
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "explanation evidence");
    for receipt in receipts.iter().take(3) {
        let artifact = receipt
            .get("artifact_ref")
            .and_then(Value::as_str)
            .unwrap_or("(none)");
        let _ = writeln!(
            out,
            "  - [{}] txn {} rev {} -> {}",
            str_at(receipt, "surface"),
            int_at(receipt, "transaction_id"),
            int_at(receipt, "revision"),
            artifact
        );
    }
    if receipts.len() > 3 {
        let _ = writeln!(out, "  - ... {} more receipt(s)", receipts.len() - 3);
    }
    if let Some(call) = explain.get("llm_call").filter(|v| !v.is_null()) {
        let _ = writeln!(
            out,
            "  - llm {} / {} window={} title={:?} activity={:?}",
            str_at(call, "provider"),
            str_at(call, "model"),
            str_at(call, "window_hash"),
            str_at(call, "parsed_title"),
            str_at(call, "parsed_activity")
        );
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn bool_at(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn int_at(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests;
