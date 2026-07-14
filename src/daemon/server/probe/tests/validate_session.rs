use super::*;
use crate::state::{llm_calls::NewLlmCall, RegisterSession};

#[tokio::test]
async fn rpc_probe_validate_session_target_checks_specific_live_surfaces() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1", "room");
    seed_llm_call(&state, "s1");
    seed_status_graph(&state, "s1", "room");
    seed_subscription_graph(&state, "s1", "room");
    seed_session_watch_graph(&state, "s1", "room");

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "session:s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "session_target", "passed");
    assert_check_status(&v, "explain", "passed");
    assert_eq!(v["surface"], "status");
    assert_eq!(v["session_evidence"]["ok"], true);
    assert_eq!(v["session_evidence"]["status_found"], true);
    assert_eq!(v["session_evidence"]["watch_found"], true);
    assert_eq!(v["session_evidence"]["sub_h_owned"], true);
    assert_eq!(v["session_evidence"]["sub_d_owned"], true);
}

#[tokio::test]
async fn rpc_probe_validate_session_target_fails_alive_session_missing_surfaces() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1", "room");
    seed_llm_call(&state, "s1");

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "session:s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "session_target", "failed");
    assert_eq!(v["session_evidence"]["missing"][0], "status");
    assert!(v["session_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("alive session is missing"));
}

#[tokio::test]
async fn rpc_probe_validate_session_target_reports_missing_session_evidence() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "session:s-missing" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "session_target", "not_proven");
    assert_check_status(&v, "explain", "failed");
    assert_eq!(v["session_evidence"]["found"], false);
}

fn seed_alive_session(state: &std::sync::Arc<DaemonState>, pubkey: &str, channel_h: &str) {
    state
        .with_store(|s| {
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                pubkey: pubkey.into(),
                agent_slug: "coder".into(),
                channel_h: channel_h.into(),
                child_pid: None,
                transcript_path: None,
                now: 100,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

fn seed_llm_call(state: &std::sync::Arc<DaemonState>, pubkey: &str) {
    state
        .with_store(|s| {
            s.record_llm_call(&NewLlmCall {
                pubkey: pubkey.into(),
                window_hash: "sha256:w".into(),
                provider: "test".into(),
                model: "model".into(),
                system_prompt: "system".into(),
                transcript_slice: "transcript".into(),
                raw_response: "TITLE: T\nNOW: A".into(),
                parsed_title: Some("T".into()),
                parsed_activity: Some("A".into()),
                created_at: 100,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

fn seed_status_graph(state: &std::sync::Arc<DaemonState>, pubkey: &str, channel_h: &str) {
    state
        .status
        .lock()
        .unwrap()
        .on_session_started(
            pubkey,
            "laptop",
            "coder",
            ".",
            BTreeSet::from([channel_h.to_string()]),
            false,
            "T",
            "",
            100,
        )
        .unwrap();
}

fn seed_subscription_graph(state: &std::sync::Arc<DaemonState>, pubkey: &str, channel_h: &str) {
    let mut sessions = BTreeMap::new();
    sessions.insert(pubkey.to_string(), BTreeSet::from([channel_h.to_string()]));
    state
        .subs
        .lock()
        .unwrap()
        .sync(&CoverageSnapshot {
            daemon_channels: BTreeSet::new(),
            addressed_pubkeys: BTreeSet::new(),
            archived_channels: BTreeSet::new(),
            sessions,
        })
        .unwrap();
}

fn seed_session_watch_graph(state: &std::sync::Arc<DaemonState>, pubkey: &str, channel_h: &str) {
    state
        .session_watch
        .lock()
        .unwrap()
        .apply(&InputFact::SessionStarted {
            pubkey: pubkey.into(),
            channel_h: Some(channel_h.into()),
            pid: None,
            at: 100,
        })
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
