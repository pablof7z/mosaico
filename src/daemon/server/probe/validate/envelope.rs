//! JSON envelope construction for `probe validate`.

use super::target_checks::TargetChecks;
use serde_json::{json, Map, Value};

#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    raw_target: Option<&str>,
    surface: Option<String>,
    handle: Option<&str>,
    explain_handle: Option<&str>,
    capsule: Option<&str>,
    verdict: &str,
    checks: Vec<Value>,
    limitations: Vec<String>,
    oracle: Value,
    seams: Value,
    stats: Option<Value>,
    stats_error: Option<String>,
    surface_state: Option<Value>,
    state_evidence: Option<Value>,
    session_consistency: Option<Value>,
    surface_states: Vec<Value>,
    state_error: Option<String>,
    why: Option<Value>,
    explanation: Option<Value>,
    explain_error: Option<String>,
    parameter_evidence: Vec<Value>,
    target_evidence: Option<Value>,
    cause_label_evidence: Option<Value>,
    target_checks: TargetChecks,
    fact_evidence: Option<Value>,
    simulation: Option<Value>,
    simulate_error: Option<String>,
    acid: Option<Value>,
    acid_error: Option<String>,
    replay: Option<Value>,
    replay_error: Option<String>,
) -> Value {
    object(vec![
        ("verb", json!("validate")),
        ("target", json!(raw_target)),
        ("surface", json!(surface)),
        ("handle", json!(handle)),
        ("explain_handle", json!(explain_handle)),
        ("capsule", json!(capsule)),
        ("ok", json!(verdict != "failed")),
        ("verdict", json!(verdict)),
        ("checks", json!(checks)),
        ("limitations", json!(limitations)),
        ("oracle", oracle),
        ("seams", seams),
        ("stats", json!(stats)),
        ("stats_error", json!(stats_error)),
        ("state", json!(surface_state)),
        ("state_evidence", json!(state_evidence)),
        ("session_consistency", json!(session_consistency)),
        ("surface_states", json!(surface_states)),
        ("state_error", json!(state_error)),
        ("why", json!(why)),
        ("explain", json!(explanation)),
        ("explain_error", json!(explain_error)),
        ("parameter_evidence", json!(parameter_evidence)),
        ("target_evidence", json!(target_evidence)),
        ("cause_label_evidence", json!(cause_label_evidence)),
        ("channel_evidence", json!(target_checks.channel_evidence)),
        ("commit_evidence", json!(target_checks.commit_evidence)),
        ("coverage_evidence", json!(target_checks.coverage_evidence)),
        ("alias_evidence", json!(target_checks.alias_evidence)),
        (
            "workspace_evidence",
            json!(target_checks.workspace_evidence),
        ),
        (
            "membership_evidence",
            json!(target_checks.membership_evidence),
        ),
        (
            "membership_snapshot_evidence",
            json!(target_checks.membership_snapshot_evidence),
        ),
        (
            "awareness_evidence",
            json!(target_checks.awareness_evidence),
        ),
        ("event_evidence", json!(target_checks.event_evidence)),
        ("inbox_evidence", json!(target_checks.inbox_evidence)),
        ("joined_evidence", json!(target_checks.joined_evidence)),
        (
            "quarantine_evidence",
            json!(target_checks.quarantine_evidence),
        ),
        ("message_evidence", json!(target_checks.message_evidence)),
        (
            "recipient_evidence",
            json!(target_checks.recipient_evidence),
        ),
        (
            "readiness_attempt_evidence",
            json!(target_checks.readiness_attempt_evidence),
        ),
        ("identity_evidence", json!(target_checks.identity_evidence)),
        (
            "hook_context_evidence",
            json!(target_checks.hook_context_evidence),
        ),
        ("llm_evidence", json!(target_checks.llm_evidence)),
        ("txn_evidence", json!(target_checks.txn_evidence)),
        ("receipt_evidence", json!(target_checks.receipt_evidence)),
        (
            "subscription_evidence",
            json!(target_checks.subscription_evidence),
        ),
        ("turn_evidence", json!(target_checks.turn_evidence)),
        ("cursor_evidence", json!(target_checks.cursor_evidence)),
        ("session_evidence", json!(target_checks.session_evidence)),
        ("status_evidence", json!(target_checks.status_evidence)),
        ("outbox_evidence", json!(target_checks.outbox_evidence)),
        (
            "session_start_evidence",
            json!(target_checks.session_start_evidence),
        ),
        (
            "session_watch_evidence",
            json!(target_checks.session_watch_evidence),
        ),
        ("fact_evidence", json!(fact_evidence)),
        ("simulate", json!(simulation)),
        ("simulate_error", json!(simulate_error)),
        ("acid", json!(acid)),
        ("acid_error", json!(acid_error)),
        ("replay", json!(replay)),
        ("replay_error", json!(replay_error)),
    ])
}

fn object(fields: Vec<(&'static str, Value)>) -> Value {
    let mut map = Map::with_capacity(fields.len());
    for (key, value) in fields {
        map.insert(key.to_string(), value);
    }
    Value::Object(map)
}
