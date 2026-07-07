use super::*;
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_all_checks_every_surface_state() {
    let state = DaemonState::new_for_test().await;

    let implicit = rpc_probe(&state, &json!({ "verb": "validate" })).unwrap();
    assert_global_validation(&implicit);
    assert!(implicit["target"].is_null());

    let explicit = rpc_probe(&state, &json!({ "verb": "validate", "target": "all" })).unwrap();
    assert_global_validation(&explicit);
    assert_eq!(explicit["target"], "all");
    assert!(explicit["target_evidence"].is_null());
    assert_no_check(&explicit, "target");
}

#[tokio::test]
async fn rpc_probe_validate_all_fails_alive_session_missing_surface_evidence() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1", "room");

    let v = rpc_probe(&state, &json!({ "verb": "validate" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_eq!(v["verdict"], "failed");
    assert_check_status(&v, "session_consistency", "failed");
    assert_eq!(v["session_consistency"]["failed_count"], 1);
    assert!(v["session_consistency"]["rows"][0]["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "status"));
}

#[tokio::test]
async fn rpc_probe_validate_all_reports_startup_warmup_before_failing_sessions() {
    let state = DaemonState::new_for_test_with_started_at(crate::util::now_secs()).await;
    seed_alive_session(&state, "s1", "room");

    let v = rpc_probe(&state, &json!({ "verb": "validate" })).unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["verdict"], "passed_with_limitations");
    assert_check_status(&v, "session_consistency", "not_proven");
    assert_eq!(v["session_consistency"]["failed_count"], 1);
    assert_eq!(v["session_consistency"]["warmup_suspected"], true);
    assert_eq!(v["session_consistency"]["live_projection_count"], 0);
    assert!(v["session_consistency"]["reason"]
        .as_str()
        .unwrap()
        .contains("daemon just started"));
}

#[tokio::test]
async fn rpc_probe_validate_all_passes_aligned_alive_session_surfaces() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1", "room");
    seed_status_graph(&state, "s1", "room");
    seed_subscription_graph(&state, "s1", "room");
    seed_session_watch_graph(&state, "s1", "room");

    let v = rpc_probe(&state, &json!({ "verb": "validate" })).unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "session_consistency", "passed");
    assert_eq!(v["session_consistency"]["session_count"], 1);
    assert_eq!(v["session_consistency"]["failed_count"], 0);
}

#[tokio::test]
async fn rpc_probe_validate_state_surface_alias_checks_surface_state() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "state:status" }),
    )
    .unwrap();

    assert_eq!(v["target"], "state:status");
    assert_eq!(v["surface"], "status");
    assert!(v["target_evidence"].is_null());
    assert_no_check(&v, "target");
    assert_check_status(&v, "state", "not_proven");
    assert_eq!(v["state"]["check_status"], "not_proven");
    assert_eq!(v["state"]["row_count"], 0);
    assert_eq!(v["state"]["sample_targets"], json!([]));
}

fn assert_global_validation(v: &serde_json::Value) {
    assert_eq!(v["ok"], true);
    assert_eq!(v["verdict"], "passed_with_limitations");
    assert!(v["surface"].is_null());
    assert!(v["state"].is_null());
    assert_eq!(v["surface_states"].as_array().unwrap().len(), 9);
    assert_check_status(v, "resource_accounting", "passed");

    for surface in [
        "status",
        "subscriptions",
        "hook_context",
        "turn_lifecycle",
        "cursor",
        "delivery",
        "session_start",
        "session_watch",
        "outbox",
    ] {
        assert_check_status(v, &format!("state:{surface}"), "not_proven");
    }
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

fn assert_no_check(v: &serde_json::Value, name: &str) {
    assert!(v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["name"] != name));
}

fn seed_alive_session(state: &std::sync::Arc<DaemonState>, session_id: &str, channel_h: &str) {
    state
        .with_store(|s| {
            s.upsert_session_row(
                session_id,
                &RegisterSession {
                    harness: "codex".into(),
                    external_id_kind: "harness_session".into(),
                    external_id: format!("native-{session_id}"),
                    agent_pubkey: "pk1".into(),
                    agent_slug: "coder".into(),
                    channel_h: channel_h.into(),
                    child_pid: None,
                    transcript_path: None,
                    resume_id: String::new(),
                    now: 100,
                },
            )?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

fn seed_status_graph(state: &std::sync::Arc<DaemonState>, session_id: &str, channel_h: &str) {
    state
        .status
        .lock()
        .unwrap()
        .on_session_started(
            session_id,
            "laptop",
            "coder",
            "pk1",
            ".",
            BTreeSet::from([channel_h.to_string()]),
            false,
            "T",
            "",
            100,
        )
        .unwrap();
}

fn seed_subscription_graph(state: &std::sync::Arc<DaemonState>, session_id: &str, channel_h: &str) {
    let mut sessions = BTreeMap::new();
    sessions.insert(
        session_id.to_string(),
        BTreeSet::from([channel_h.to_string()]),
    );
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

fn seed_session_watch_graph(
    state: &std::sync::Arc<DaemonState>,
    session_id: &str,
    channel_h: &str,
) {
    state
        .session_watch
        .lock()
        .unwrap()
        .apply(&InputFact::SessionStarted {
            session_id: session_id.into(),
            channel_h: Some(channel_h.into()),
            agent_pubkey: Some("pk1".into()),
            pid: None,
            at: 100,
        })
        .unwrap();
}
