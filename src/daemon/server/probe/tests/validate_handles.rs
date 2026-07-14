use super::*;
use crate::reconcile::{CursorSeed, SessionStartRequestFact, TurnProjectionSeed};

mod labels;

#[tokio::test]
async fn rpc_probe_validate_accepts_visible_trellis_resource_paths() {
    let state = DaemonState::new_for_test().await;
    seed_visible_path_state(&state);

    let status_state = rpc_probe(&state, &json!({ "verb": "state", "surface": "status" })).unwrap();
    assert_eq!(status_state["rows"][0]["resource_key"], "status/s1");

    let outbox_state = rpc_probe(&state, &json!({ "verb": "state", "surface": "outbox" })).unwrap();
    assert_eq!(outbox_state["rows"][0]["resource_key"], "outbox/7");

    let cases = [
        ("status/s1/activity", "status", "status/s1"),
        ("sub/h/room", "subscriptions", "sub/h/room"),
        ("sub/d/room", "subscriptions", "sub/d/room"),
        ("sub/p/pk1", "subscriptions", "sub/p/pk1"),
        (
            "turn_lifecycle/s1/turn_started",
            "turn_lifecycle",
            "turn_lifecycle/s1",
        ),
        ("cursor/s1/observed_cursor", "cursor", "cursor/s1"),
        ("outbox/7/event_id", "outbox", "outbox/7"),
        (
            "session_start/s1/request",
            "session_start",
            "session_start/s1",
        ),
        ("session-watch/s1", "session_watch", "session-watch/s1"),
    ];

    for (target, surface, resource_key) in cases {
        let v = rpc_probe(&state, &json!({ "verb": "validate", "target": target })).unwrap();
        assert_eq!(v["target"], target);
        assert_eq!(v["surface"], surface);
        assert_check_status(&v, "why", "passed");
        assert_eq!(v["why"]["resource_key"], resource_key);
    }
}

#[tokio::test]
async fn rpc_probe_validate_slash_status_handle_keeps_acid_evidence() {
    let state = DaemonState::new_for_test().await;
    seed_visible_path_state(&state);

    let v = rpc_probe(
        &state,
        &json!({
            "verb": "validate",
            "target": "status/s1/activity",
            "fact": {
                "StatusDrive": {
                    "DistillCompleted": {
                        "pubkey": "s1",
                        "title": "T",
                        "activity": "compiling",
                        "window_hash": "sha256:w2",
                        "at": 130
                    }
                }
            }
        }),
    )
    .unwrap();

    assert_eq!(v["surface"], "status");
    assert_check_status(&v, "why", "passed");
    assert_check_status(&v, "simulate", "passed");
    assert_check_status(&v, "acid", "passed");
    assert_eq!(v["acid"]["handle"], "status/s1/activity");
    assert_eq!(v["acid"]["cause"], "status/s1/activity");
}

#[tokio::test]
async fn rpc_probe_validate_generic_handles_include_matched_state_evidence() {
    let state = DaemonState::new_for_test().await;
    seed_visible_path_state(&state);

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "sub:room" })).unwrap();

    assert_eq!(v["surface"], "subscriptions");
    assert_check_status(&v, "state", "passed");
    assert_eq!(v["subscription_evidence"]["kind"], "channel");
    assert_eq!(v["subscription_evidence"]["expected_resource_count"], 2);
    assert_eq!(v["subscription_evidence"]["found_resource_count"], 2);
    assert_eq!(
        v["subscription_evidence"]["resources"][0]["resource_key"],
        "sub/h/room"
    );
}

#[tokio::test]
async fn rpc_probe_validate_specific_missing_handle_fails_state_check() {
    let state = DaemonState::new_for_test().await;
    seed_visible_path_state(&state);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "status/missing/activity" }),
    )
    .unwrap();

    assert_eq!(v["surface"], "status");
    assert_check_status(&v, "why", "failed");
    assert_check_status(&v, "state", "failed");
    assert!(check_summary(&v, "state").contains("status/missing"));
}

fn seed_visible_path_state(state: &std::sync::Arc<DaemonState>) {
    {
        let mut r = state.status.lock().expect("status mutex");
        r.on_session_started(
            "s1",
            "laptop",
            "coder",
            ".",
            BTreeSet::from(["room".to_string()]),
            true,
            "T",
            "reading",
            100,
        )
        .unwrap();
        r.on_distill("s1", "T", "reviewing", 110).unwrap();
    }

    {
        let mut r = state.subs.lock().expect("subs mutex");
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
        r.sync(&CoverageSnapshot {
            daemon_channels: BTreeSet::from(["room".to_string()]),
            addressed_pubkeys: BTreeSet::from(["pk1".to_string()]),
            archived_channels: BTreeSet::new(),
            sessions,
        })
        .unwrap();
    }

    state
        .turn_lifecycle
        .lock()
        .expect("turn lifecycle mutex")
        .on_turn_started(
            TurnProjectionSeed {
                pubkey: "s1".into(),
                working: false,
                turn_started_at: 0,
                transcript_ref: None,
            },
            100,
            None,
        )
        .unwrap();

    state
        .cursor
        .lock()
        .expect("cursor mutex")
        .request(
            CursorSeed {
                pubkey: "s1".into(),
                seen_cursor: 10,
            },
            InputFact::TurnCheckRequested {
                pubkey: "s1".into(),
                observed_cursor: 10,
                working: true,
                at: 120,
            },
        )
        .unwrap();

    state
        .outbox
        .lock()
        .expect("outbox mutex")
        .drive(InputFact::OutboxEnqueueApplied {
            local_id: 7,
            event_id: "ev7".into(),
            event_hash: "sha256:event".into(),
            source_surface: "status".into(),
            source_ref: "status/s1#tx:1".into(),
            at: 100,
        })
        .unwrap();

    state
        .session_start
        .lock()
        .expect("session_start mutex")
        .drive(InputFact::SessionStartRequested(session_start_request()))
        .unwrap();

    state
        .session_watch
        .lock()
        .expect("session watch mutex")
        .apply(&InputFact::SessionStarted {
            pubkey: "s1".into(),
            channel_h: Some("room".into()),
            pid: Some(42),
            at: 100,
        })
        .unwrap();
}

fn session_start_request() -> SessionStartRequestFact {
    SessionStartRequestFact {
        pubkey: "s1".into(),
        agent: "coder".into(),
        harness: "codex".into(),
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
        signer_label: "coder".into(),
        already_running: false,
        channel_already_subscribed: false,
        at: 100,
    }
}

fn assert_check_status(v: &serde_json::Value, name: &str, status: &str) {
    assert_eq!(check_row(v, name)["status"], status);
}

fn check_summary(v: &serde_json::Value, name: &str) -> String {
    check_row(v, name)["summary"].as_str().unwrap().to_string()
}

fn check_row<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    let check = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .expect("check row");
    check
}

fn assert_no_check(v: &serde_json::Value, name: &str) {
    assert!(!v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["name"] == name));
}
