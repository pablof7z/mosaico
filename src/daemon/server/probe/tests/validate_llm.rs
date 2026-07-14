use super::*;
use crate::instrument::{changed_summary_json, window_hash};
use crate::state::llm_calls::NewLlmCall;
use crate::state::receipts::NewReceipt;
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_llm_target_checks_call_and_joined_receipt() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1");
    seed_status_graph(&state, "s1");
    let (id, event_id) = seed_llm_call(&state, "s1", true);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": format!("llm:{id}") }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["surface"], "status");
    assert_check_status(&v, "llm_outcome", "passed");
    assert_check_status(&v, "explain", "passed");
    assert_check_status(&v, "state", "passed");
    assert_eq!(v["llm_evidence"]["call_found"], true);
    assert_eq!(v["llm_evidence"]["session_row_found"], true);
    assert_eq!(v["llm_evidence"]["receipt_count"], 1);
    assert_eq!(v["llm_evidence"]["receipt_artifacts"][0], event_id);
}

#[tokio::test]
async fn rpc_probe_validate_llm_target_reports_missing_joined_receipt_as_limitation() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1");
    let (id, _) = seed_llm_call(&state, "s1", false);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": format!("llm:{id}") }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["verdict"], "passed_with_limitations");
    assert_check_status(&v, "llm_outcome", "passed");
    assert_check_status(&v, "explain", "passed");
    assert_eq!(v["llm_evidence"]["receipt_count"], 0);
    assert!(v["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("no status receipt")));
}

#[tokio::test]
async fn rpc_probe_validate_llm_target_fails_missing_call() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "llm:999" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "llm_outcome", "not_proven");
    assert_check_status(&v, "explain", "failed");
    assert_eq!(v["llm_evidence"]["call_found"], false);
}

fn seed_llm_call(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    with_receipt: bool,
) -> (i64, String) {
    let wh = window_hash("transcript slice");
    let event_id = "evt-status".to_string();
    let id = state
        .with_store(|s| {
            let id = s.record_llm_call(&NewLlmCall {
                pubkey: pubkey.into(),
                window_hash: wh.clone(),
                provider: "ollama".into(),
                model: "glm".into(),
                system_prompt: "system".into(),
                transcript_slice: "transcript slice".into(),
                raw_response: "TITLE: T\nNOW: A".into(),
                parsed_title: Some("T".into()),
                parsed_activity: Some("A".into()),
                created_at: 100,
            })?;
            if with_receipt {
                s.record_receipt(&NewReceipt {
                    surface: "status".into(),
                    transaction_id: 7,
                    revision: 3,
                    changed_summary: changed_summary_json(&[], &[], &[], Some(pubkey), Some(&wh)),
                    commands: format!(r#"[{{"kind":"replace","key":"status/{pubkey}"}}]"#),
                    artifact_ref: Some(event_id.clone()),
                    created_at: 101,
                })?;
            }
            Ok::<_, anyhow::Error>(id)
        })
        .unwrap();
    (id, event_id)
}

fn seed_alive_session(state: &std::sync::Arc<DaemonState>, pubkey: &str) {
    state
        .with_store(|s| {
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                pubkey: pubkey.into(),
                agent_slug: "coder".into(),
                channel_h: "room".into(),
                child_pid: None,
                transcript_path: None,
                now: 100,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

fn seed_status_graph(state: &std::sync::Arc<DaemonState>, pubkey: &str) {
    state
        .status
        .lock()
        .unwrap()
        .on_session_started(
            pubkey,
            "laptop",
            "coder",
            ".",
            BTreeSet::from(["room".to_string()]),
            false,
            "T",
            "A",
            100,
        )
        .unwrap();
}

fn assert_check_status(v: &serde_json::Value, name: &str, status: &str) {
    assert_eq!(check_row(v, name)["status"], status);
}

fn check_row<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .expect("check row")
}
