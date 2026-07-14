use super::*;
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_session_watch_checks_graph_store_and_pid() {
    let state = DaemonState::new_for_test().await;
    let now = crate::util::now_secs();
    let pid = std::process::id() as i32;
    state.with_store(|s| {
        s.reserve_session(&RegisterSession {
            harness: "codex".into(),
            pubkey: "pk-agent".into(),
            agent_slug: "coder".into(),
            channel_h: "room".into(),
            child_pid: Some(pid),
            transcript_path: None,
            now,
        })
        .unwrap();
    });
    let pubkey = "pk-agent".to_string();
    state
        .session_watch
        .lock()
        .unwrap()
        .apply(&InputFact::SessionStarted {
            pubkey: pubkey.clone(),
            channel_h: Some("room".into()),
            pid: Some(pid),
            at: now,
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": format!("watch:{pubkey}") }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "session_watch_outcome", "passed");
    assert_check_status(&v, "state", "passed");
    assert_eq!(v["session_watch_evidence"]["graph_open"], true);
    assert_eq!(v["session_watch_evidence"]["session_alive"], true);
    assert_eq!(v["session_watch_evidence"]["process_alive"], true);
}

#[tokio::test]
async fn rpc_probe_validate_session_watch_reports_graph_store_mismatch() {
    let state = DaemonState::new_for_test().await;
    state
        .session_watch
        .lock()
        .unwrap()
        .apply(&InputFact::SessionStarted {
            pubkey: "s1".into(),
            channel_h: Some("room".into()),
            pid: Some(123),
            at: 100,
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "session_watch:s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "session_watch_outcome", "failed");
    assert_eq!(v["session_watch_evidence"]["graph_open"], true);
    assert_eq!(v["session_watch_evidence"]["session_row_found"], false);
    assert!(v["session_watch_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("local session row is missing"));
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
