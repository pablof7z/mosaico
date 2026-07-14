use super::*;
use crate::fabric_context::{
    MembersInput, MessagesInput, MetaInput, PresenceInput, ReactionsInput, ViewInputs,
};
use crate::reconcile::{CoverageSnapshot, InputFact, StatusDrive};
use crate::state::trellis_commits::NewCommit;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use trellis_testing::DataTransactionScript;

mod stats;
mod validate;
mod validate_awareness;
mod validate_channel;
mod validate_commit;
mod validate_coverage;
mod validate_event;
mod validate_fact_flow;
mod validate_global;
mod validate_handles;
mod validate_hook_context;
mod validate_inbox;
mod validate_inputs;
mod validate_joined;
mod validate_llm;
mod validate_membership;
mod validate_message;
mod validate_outbox;
mod validate_projection;
mod validate_quarantine;
mod validate_readiness_attempt;
mod validate_receipt;
mod validate_recipient;
mod validate_session;
mod validate_session_watch;
mod validate_status;
mod validate_subscription;
mod validate_txn;
mod validate_workspace;

/// End-to-end proof that the `probe` RPC — the lock/param/dispatch inch in
/// `rpc_probe` — actually works over a REAL `DaemonState`, not merely that the
/// pure value-fns compile.
#[tokio::test]
async fn rpc_probe_reflects_driven_state_for_every_verb() {
    let state = DaemonState::new_for_test().await;

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
            1_700_000_010,
        )
        .unwrap();
        r.on_distill("s1", "T", "reviewing the PR", 1_700_000_010)
            .unwrap();
    }

    {
        let mut r = state.subs.lock().expect("subs mutex");
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
        r.sync(&CoverageSnapshot {
            daemon_channels: BTreeSet::from(["room".to_string()]),
            addressed_pubkeys: BTreeSet::new(),
            archived_channels: BTreeSet::new(),
            sessions,
        })
        .unwrap();
    }

    let empty_inputs = || {
        ViewInputs::from_parts(
            MetaInput::default(),
            MembersInput::default(),
            PresenceInput::default(),
            MessagesInput::default(),
            ReactionsInput::default(),
        )
    };
    crate::turn_context::render_hook_context(
        &state.hook_contexts,
        "s1",
        "turn_start",
        0,
        1_700_000_010,
        empty_inputs(),
    )
    .unwrap();
    crate::turn_context::render_hook_context(
        &state.hook_contexts,
        "s1",
        "turn_check",
        1,
        1_700_000_011,
        empty_inputs(),
    )
    .unwrap();

    let mut replay_script = DataTransactionScript::new();
    replay_script
        .step("tick")
        .operation(InputFact::StatusDrive(StatusDrive::Tick {
            pubkey: "missing".into(),
            at: 1_700_000_010,
        }))
        .commit();
    let replay_json = replay_script.to_json().unwrap();
    state.with_store(|s| {
        s.record_commit(&NewCommit {
            surface: "status".into(),
            transaction_id: 1,
            revision: 1,
            mode: "authoritative".into(),
            trigger_kind: "distill".into(),
            trigger_ref: "s1".into(),
            changed_inputs_json: "[]".into(),
            changed_derived_json: "[]".into(),
            changed_collections_json: "[]".into(),
            resource_commands_json: "[]".into(),
            output_frames_json: "[]".into(),
            command_count: 1,
            output_count: 0,
            effect_count: 1,
            suppressed_count: 0,
            noop: 0,
            oracle_status: None,
            oracle_error: None,
            duration_us: 100,
            graph_nodes: 6,
            graph_resources: 0,
            created_at: 1_700_000_010,
        })
        .unwrap();
        s.record_replay_capsule(&crate::state::trellis_replay_capsules::NewReplayCapsule {
            surface: "status".into(),
            trigger_kind: "tick".into(),
            trigger_ref: "missing".into(),
            script_json: replay_json,
            format_version: 1,
            created_at: 1_700_000_010,
        })
        .unwrap();
    });

    let oracle = rpc_probe(&state, &json!({ "verb": "oracle" })).unwrap();
    assert_eq!(oracle["ok"], true);
    assert_surface_status(&oracle, "status", "green");
    assert_surface_status(&oracle, "turn_lifecycle", "green");
    assert_surface_status(&oracle, "cursor", "green");
    assert_surface_status(&oracle, "delivery", "green");
    assert_surface_status(&oracle, "session_start", "green");
    assert_surface_status(&oracle, "session_watch", "green");
    assert_surface_status(&oracle, "outbox", "green");
    assert_surface_status(&oracle, "hook_context", "green");

    let stats = rpc_probe(&state, &json!({ "verb": "stats", "since": 0 })).unwrap();
    let sstatus = stats["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["surface"] == "status")
        .expect("status stats row");
    assert_eq!(sstatus["commits"], 1);
    assert_eq!(sstatus["effectful"], 1);

    let seams = rpc_probe(&state, &json!({ "verb": "seams" })).unwrap();
    assert_eq!(seams["host_seam_coverage_percent"], 77);
    assert_surface_mode(&seams, "status", "authoritative");
    assert_surface_mode(&seams, "turn_lifecycle", "authoritative");
    assert_surface_mode(&seams, "cursor", "authoritative");
    assert_surface_mode(&seams, "delivery", "authoritative");
    assert_surface_mode(&seams, "session_start", "advisory");
    assert_surface_mode(&seams, "session_watch", "advisory");
    assert_surface_mode(&seams, "outbox", "authoritative");
    assert_surface_mode(&seams, "hook_context", "authoritative");

    let replay = rpc_probe(
        &state,
        &json!({ "verb": "replay", "capsule": "1", "assert": true }),
    )
    .unwrap();
    assert_eq!(replay["asserted"], true);
    assert_eq!(replay["steps"], 1);

    let sim = rpc_probe(
        &state,
        &json!({
            "verb": "simulate", "surface": "status", "session": "s1",
            "activity": "compiling", "title": null, "now": null, "fact": null,
        }),
    )
    .unwrap();
    assert_eq!(sim["would_publish"], true);
    assert_eq!(sim["commands"][0]["op"], "Replace");
    assert_eq!(sim["revision_before"], sim["revision_after"]);

    let fact = InputFact::StatusDrive(StatusDrive::DistillCompleted {
        pubkey: "s1".into(),
        title: "T".into(),
        activity: "compiling".into(),
        window_hash: Some("sha256:w2".into()),
        at: 1_700_000_020,
    });
    let diff = rpc_probe(
        &state,
        &json!({ "verb": "diff", "surface": "status", "fact": fact, "capsule": null }),
    )
    .unwrap();
    assert_eq!(diff["mode"], "live-preview");

    let why = rpc_probe(&state, &json!({ "verb": "why", "handle": "status:s1" })).unwrap();
    assert_eq!(why["found"], true);
    assert_eq!(why["last_kind"], "Replace");
    assert_array_contains(&why["input_causes"], "status/s1/activity");

    let hwhy = rpc_probe(&state, &json!({ "verb": "why", "handle": "hook:s1" })).unwrap();
    assert_eq!(hwhy["found"], true);
    assert_array_contains(&hwhy["input_causes"], "hook/s1/cursor");

    let st = rpc_probe(&state, &json!({ "verb": "state", "surface": "status" })).unwrap();
    let rows = st["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["session"], "s1");
    assert_eq!(rows[0]["activity"], "reviewing the PR");

    let subs = rpc_probe(
        &state,
        &json!({ "verb": "state", "surface": "subscriptions" }),
    )
    .unwrap();
    assert!(subs["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["resource_key"] == "sub/h/room"));

    let hook = rpc_probe(
        &state,
        &json!({ "verb": "state", "surface": "hook_context", "handle": "s1", "dump": true }),
    )
    .unwrap();
    assert_eq!(hook["found"], true);
    assert_eq!(hook["rows"][0]["session"], "s1");
    assert_eq!(hook["rows"][0]["render_count"], 2);
    assert!(!hook["rows"][0]["debug_dump"].as_str().unwrap().is_empty());
}

fn assert_surface_status(v: &serde_json::Value, surface: &str, status: &str) {
    let row = v["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["surface"] == surface)
        .expect("surface row");
    assert_eq!(row["status"], status);
}

fn assert_surface_mode(v: &serde_json::Value, surface: &str, mode: &str) {
    assert!(v["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["surface"] == surface && r["mode"] == mode));
}

fn assert_array_contains(v: &serde_json::Value, needle: &str) {
    assert!(v.as_array().unwrap().iter().any(|l| l == needle));
}
