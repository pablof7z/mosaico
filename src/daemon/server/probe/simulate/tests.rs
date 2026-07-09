use super::*;
use crate::fabric_context::ViewInputs;
use crate::reconcile::{
    CoverageSnapshot, HookContextRenderFact, SessionStartRequestFact, StatusSessionStartedArgs,
};
use std::collections::{BTreeMap, BTreeSet};

#[tokio::test]
async fn simulate_status_accepts_input_fact_json_without_mutating() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.status.lock().unwrap();
        r.on_session_started(
            "s1",
            "laptop",
            "coder",
            "pk1",
            ".",
            BTreeSet::from(["room".to_string()]),
            true,
            "T",
            "reading",
            100,
        )
        .unwrap();
    }
    let fact = InputFact::StatusDrive(StatusDrive::DistillCompleted {
        session_id: "s1".into(),
        title: "T".into(),
        activity: "compiling".into(),
        window_hash: None,
        at: 100,
    });

    let out = simulate_value(
        &state,
        &json!({ "verb": "simulate", "surface": "status", "fact": fact }),
    )
    .unwrap();

    assert_eq!(out["would_publish"], true);
    assert_eq!(out["commands"][0]["op"], "Replace");
    assert_eq!(out["revision_before"], out["revision_after"]);
}

#[tokio::test]
async fn simulate_subscriptions_accepts_input_fact_json_without_mutating() {
    let state = DaemonState::new_for_test().await;
    let mut sessions = BTreeMap::new();
    sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
    let fact = InputFact::SubscriptionSync {
        snapshot: CoverageSnapshot {
            daemon_channels: BTreeSet::from(["room".to_string()]),
            addressed_pubkeys: BTreeSet::new(),
            archived_channels: BTreeSet::new(),
            sessions,
        },
        at: 100,
    };

    let out = simulate_value(
        &state,
        &json!({ "verb": "simulate", "surface": "subscriptions", "fact": fact }),
    )
    .unwrap();

    assert_eq!(out["would_effect"], true);
    assert_eq!(out["commands"][0]["op"], "Open");
    assert_eq!(out["revision_before"], out["revision_after"]);
}

#[tokio::test]
async fn simulate_cursor_accepts_input_fact_json_without_mutating() {
    let state = DaemonState::new_for_test().await;
    let fact = InputFact::TurnCheckRequested {
        session_id: "s1".into(),
        observed_cursor: 10,
        working: true,
        at: 20,
    };

    let out = simulate_value(&state, &json!({ "verb": "simulate", "fact": fact })).unwrap();

    assert_eq!(out["surface"], "cursor");
    assert_eq!(out["would_effect"], true);
    assert_eq!(out["commands"][0]["op"], "Open");
    assert!(out["changed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "cursor/s1/observed_cursor"));
    assert_eq!(out["revision_before"], out["revision_after"]);
}

#[tokio::test]
async fn simulate_outbox_accepts_input_fact_json_without_mutating() {
    let state = DaemonState::new_for_test().await;
    let fact = InputFact::OutboxEnqueueApplied {
        local_id: 7,
        event_id: "ev7".into(),
        event_hash: "sha256:event".into(),
        source_surface: "status".into(),
        source_ref: "status/s1#tx:1".into(),
        at: 100,
    };

    let out = simulate_value(&state, &json!({ "verb": "simulate", "fact": fact })).unwrap();

    assert_eq!(out["surface"], "outbox");
    assert_eq!(out["would_effect"], true);
    assert_eq!(out["commands"][0]["op"], "Open");
    assert!(out["changed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "outbox/7/event_id"));
    assert_eq!(out["revision_before"], out["revision_after"]);
}

#[tokio::test]
async fn simulate_new_status_session_labels_preview_only_nodes() {
    let state = DaemonState::new_for_test().await;
    let fact = InputFact::StatusDrive(StatusDrive::SessionStarted(StatusSessionStartedArgs {
        session_id: "s1".into(),
        host: "h".into(),
        slug: "a".into(),
        pubkey: "pk".into(),
        rel_cwd: ".".into(),
        channels: BTreeSet::from(["room".to_string()]),
        working: true,
        title: "T".into(),
        activity: "reading".into(),
        at: 100,
    }));

    let out = simulate_value(&state, &json!({ "verb": "simulate", "fact": fact })).unwrap();

    assert!(out["changed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "status/s1/activity"));
    assert_eq!(out["revision_before"], out["revision_after"]);
}

mod extended;
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
            "workspace": { "name": "", "about": "" },
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

fn session_start_request() -> SessionStartRequestFact {
    SessionStartRequestFact {
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
