use super::*;
use crate::fabric_context::ViewInputs;
use crate::reconcile::InputFact;
use serde_json::json;

#[tokio::test]
async fn rpc_probe_validate_explains_session_start_handles() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.session_start.lock().expect("session_start mutex");
        r.drive(InputFact::SessionStartRequested(session_start_request()))
            .unwrap();
    }

    let validation = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "session_start:s1" }),
    )
    .unwrap();

    assert_eq!(validation["surface"], "session_start");
    assert_eq!(validation["verdict"], "passed_with_limitations");
    assert_check_status(&validation, "seams", "not_proven");
    assert_check_status(&validation, "why", "passed");
    assert_eq!(validation["why"]["kind"], "session_start");
    assert_eq!(validation["why"]["resource_key"], "session_start/s1");

    let fact_validation = rpc_probe(
        &state,
        &json!({
            "verb": "validate",
            "fact": { "SessionStartRequested": session_start_request() }
        }),
    )
    .unwrap();

    assert_eq!(fact_validation["surface"], "session_start");
    assert_check_status(&fact_validation, "simulate", "passed");
    assert_eq!(fact_validation["simulate"]["surface"], "session_start");
    assert_eq!(fact_validation["verdict"], "passed_with_limitations");

    seed_hook_context_graph(&state);
    let hook_validation = rpc_probe(
        &state,
        &json!({ "verb": "validate", "fact": hook_context_fact() }),
    )
    .unwrap();

    assert_eq!(hook_validation["surface"], "hook_context");
    assert_check_status(&hook_validation, "seams", "passed");
    assert_check_status(&hook_validation, "simulate", "passed");
    assert_eq!(hook_validation["simulate"]["output_frames"], 1);
}

#[tokio::test]
async fn rpc_probe_validate_reports_session_start_failure_stage() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.session_start.lock().expect("session_start mutex");
        r.drive(InputFact::SessionStartRequested(session_start_request()))
            .unwrap();
        r.drive(InputFact::SessionStartFailed(
            crate::reconcile::SessionStartFailedFact {
                session_id: "s1".into(),
                stage: "channel_ready".into(),
                error: "relay rejected event: timeout".into(),
                at: 101,
            },
        ))
        .unwrap();
    }

    let validation = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "session_start:s1" }),
    )
    .unwrap();

    assert_eq!(validation["ok"], false);
    assert_eq!(validation["verdict"], "failed");
    assert_check_status(&validation, "session_start_outcome", "failed");
    assert_check_status(&validation, "state", "passed");
    assert_eq!(
        validation["session_start_evidence"]["action"],
        "RecordFailed"
    );
    assert_eq!(
        validation["session_start_evidence"]["failure_stage"],
        "channel_ready"
    );
    assert!(validation["session_start_evidence"]["failure_error"]
        .as_str()
        .unwrap()
        .contains("timeout"));
}

#[tokio::test]
async fn rpc_probe_validate_reports_missing_live_context_as_not_proven() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "fact": hook_context_fact() }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["verdict"], "passed_with_limitations");
    assert_eq!(v["surface"], "hook_context");
    assert_check_status(&v, "fact", "passed");
    assert_check_status(&v, "simulate", "not_proven");
    assert!(v["simulate"].is_null());
    assert!(v["simulate_error"]
        .as_str()
        .unwrap()
        .contains("has not rendered"));
}

#[tokio::test]
async fn rpc_probe_validate_process_exit_through_session_watch() {
    let state = DaemonState::new_for_test().await;
    let pid = std::process::id() as i32;
    state
        .with_store(|s| {
            s.upsert_session_row(
                "s1",
                &crate::state::RegisterSession {
                    harness: "codex".into(),
                    external_id_kind: "harness_session".into(),
                    external_id: "native-1".into(),
                    agent_pubkey: "pk".into(),
                    agent_slug: "coder".into(),
                    channel_h: "room".into(),
                    child_pid: Some(pid),
                    transcript_path: None,
                    resume_id: String::new(),
                    now: 100,
                },
            )
        })
        .unwrap();
    state
        .session_watch
        .lock()
        .unwrap()
        .apply(&InputFact::SessionStarted {
            session_id: "s1".into(),
            channel_h: Some("room".into()),
            agent_pubkey: Some("pk".into()),
            pid: Some(pid),
            at: 100,
        })
        .unwrap();

    let validation = rpc_probe(
        &state,
        &json!({
            "verb": "validate",
            "target": "watch:s1",
            "fact": {
                "ProcessExited": {
                    "session_id": "s1",
                    "pid": pid,
                    "at": 101
                }
            }
        }),
    )
    .unwrap();

    assert_eq!(validation["surface"], "session_watch");
    assert_eq!(validation["verdict"], "passed_with_limitations");
    assert_check_status(&validation, "fact", "passed");
    assert_check_status(&validation, "seams", "not_proven");
    assert_check_status(&validation, "why", "passed");
    assert_check_status(&validation, "simulate", "passed");
    assert_eq!(validation["simulate"]["surface"], "session_watch");
    assert_eq!(validation["simulate"]["commands"][0]["op"], "Close");
    assert_eq!(validation["why"]["kind"], "session_watch");
}

fn session_start_request() -> crate::reconcile::SessionStartRequestFact {
    crate::reconcile::SessionStartRequestFact {
        session_id: "s1".into(),
        agent: "coder".into(),
        harness: "codex".into(),
        external_id_kind: "harness_session".into(),
        external_id: "native-1".into(),
        native_id: "native-1".into(),
        work_root: "/repo".into(),
        channel_h: "room".into(),
        channel_for_upsert: "room".into(),
        rel_cwd: ".".into(),
        room_parent: None,
        channel_provision_name: None,
        watch_pid: Some(42),
        pty_session: Some("%1".into()),
        ring_doorbell: true,
        signer_pubkey: "base".into(),
        signer_label: "coder".into(),
        already_running: false,
        channel_already_subscribed: false,
        at: 100,
    }
}

fn hook_context_fact() -> InputFact {
    InputFact::HookContextRender(crate::reconcile::HookContextRenderFact {
        session_id: "s1".into(),
        hook_kind: "turn_start".into(),
        cursor: 0,
        now: 100,
        force: false,
        emitted_text_hash: None,
        inputs_json: hook_inputs_json(&["probe warning"]),
    })
}

fn seed_hook_context_graph(state: &std::sync::Arc<DaemonState>) {
    let inputs: ViewInputs = serde_json::from_value(hook_inputs_json(&[])).unwrap();
    state
        .hook_contexts
        .lock()
        .unwrap()
        .entry("s1".into())
        .or_default()
        .render_context("s1", "turn_start", 0, 99, inputs)
        .unwrap();
}

fn hook_inputs_json(warnings: &[&str]) -> serde_json::Value {
    json!({
        "meta": {
            "self_row": null,
            "project": { "name": "", "about": "" },
            "agents": [],
            "channels": [],
            "unjoined": [],
            "warnings": warnings,
            "self_pubkey": "",
            "self_ref": "",
            "force": false
        },
        "members": { "roster": {}, "refs": {}, "backend": [] },
        "presence": { "statuses": {} },
        "messages": { "channels": {} }
    })
}

fn assert_check_status(v: &serde_json::Value, name: &str, status: &str) {
    let row = check_row(v, name);
    assert_eq!(row["status"], status);
}

fn check_row<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .expect("check row")
}
