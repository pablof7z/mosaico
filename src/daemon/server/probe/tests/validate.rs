use super::*;
use crate::instrument::{changed_summary_json, window_hash};
use crate::state::llm_calls::NewLlmCall;
use crate::state::receipts::NewReceipt;
use crate::state::RegisterSession;
use serde_json::json;
use std::collections::BTreeSet;
use trellis_testing::DataTransactionScript;

mod acid;

#[tokio::test]
async fn rpc_probe_validate_checks_status_fact_and_capsule() {
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
    state
        .with_store(|s| {
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                pubkey: "s1".into(),
                agent_slug: "coder".into(),
                channel_h: "room".into(),
                child_pid: None,
                transcript_path: None,
                now: 1_700_000_010,
            })?;
            s.set_working("s1", true, 1_700_000_010)?;
            s.set_session_distill("s1", "T", "reviewing the PR", 1_700_000_010)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
    {
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
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
    state
        .session_watch
        .lock()
        .unwrap()
        .apply(&InputFact::SessionStarted {
            pubkey: "s1".into(),
            channel_h: Some("room".into()),
            pid: None,
            at: 1_700_000_010,
        })
        .unwrap();
    seed_replay_capsule(&state);

    let validation = rpc_probe(
        &state,
        &json!({
            "verb": "validate",
            "target": "status:s1",
            "fact": {
                "StatusDrive": {
                    "DistillCompleted": {
                        "pubkey": "s1",
                        "title": "T",
                        "activity": "compiling",
                        "window_hash": "sha256:w2",
                        "at": 1_700_000_020
                    }
                }
            },
            "since": 0
        }),
    )
    .unwrap();
    assert_eq!(validation["ok"], true);
    assert_eq!(validation["verdict"], "passed");
    assert_eq!(validation["surface"], "status");
    assert_check_status(&validation, "oracle", "passed");
    assert_check_status(&validation, "seams", "passed");
    assert_check_status(&validation, "why", "passed");
    assert_check_status(&validation, "simulate", "passed");
    assert_check_status(&validation, "acid", "passed");
    let seams_summary = check_row(&validation, "seams")["summary"].as_str().unwrap();
    assert!(seams_summary.contains("status seam is authoritative"));

    let fact_only = rpc_probe(
        &state,
        &json!({
            "verb": "validate",
            "fact": {
                "StatusDrive": {
                    "Tick": {
                        "pubkey": "s1",
                        "at": 1_700_000_030
                    }
                }
            },
            "since": 0
        }),
    )
    .unwrap();
    assert_eq!(fact_only["surface"], "status");
    assert_eq!(fact_only["simulate"]["surface"], "status");
    assert_eq!(fact_only["state"]["surface"], "status");

    let replay_validation = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "capsule:1" }),
    )
    .unwrap();
    assert_eq!(replay_validation["ok"], true);
    assert_eq!(replay_validation["surface"], "status");
    assert_check_status(&replay_validation, "seams", "passed");
    assert_check_status(&replay_validation, "replay", "passed");

    let global = rpc_probe(&state, &json!({ "verb": "validate" })).unwrap();
    assert_eq!(global["verdict"], "passed_with_limitations");
    assert_check_status(&global, "seams", "not_proven");
    assert!(check_row(&global, "seams")["summary"]
        .as_str()
        .unwrap()
        .contains("session_watch (advisory)"));
}

#[tokio::test]
async fn rpc_probe_validate_explains_published_artifact_handles() {
    let state = DaemonState::new_for_test().await;
    let wh = window_hash("transcript slice");
    state
        .with_store(|s| {
            s.record_llm_call(&NewLlmCall {
                pubkey: "s1".into(),
                window_hash: wh.clone(),
                provider: "ollama".into(),
                model: "glm".into(),
                system_prompt: "system".into(),
                transcript_slice: "transcript slice".into(),
                raw_response: "TITLE: T\nNOW: A".into(),
                parsed_title: Some("T".into()),
                parsed_activity: Some("A".into()),
                created_at: 1_700_000_010,
            })?;
            s.record_receipt(&NewReceipt {
                surface: "status".into(),
                transaction_id: 7,
                revision: 3,
                changed_summary: changed_summary_json(&[], &[], &[], Some("s1"), Some(&wh)),
                commands: r#"[{"kind":"replace","key":"status/s1"}]"#.into(),
                artifact_ref: Some("evt-status".into()),
                created_at: 1_700_000_011,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event:evt-status" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["surface"], "status");
    assert_eq!(v["explain_handle"], "event:evt-status");
    assert_check_status(&v, "explain", "passed");
    assert_check_status(&v, "resource_accounting", "passed");
    assert_eq!(v["explain"]["receipts"][0]["transaction_id"], 7);
    assert_eq!(v["explain"]["llm_call"]["parsed_activity"], "A");

    let session = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "session:s-missing" }),
    )
    .unwrap();
    assert_eq!(session["surface"], "status");
    assert_check_status(&session, "explain", "failed");
    assert_check_status(&session, "state", "not_proven");
}

#[tokio::test]
async fn rpc_probe_validate_reports_stats_errors_inside_envelope() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| s.drop_trellis_commits_for_test())
        .unwrap();

    let v = rpc_probe(&state, &json!({ "verb": "validate" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_eq!(v["verdict"], "failed");
    assert_check_status(&v, "oracle", "passed");
    assert_check_status(&v, "resource_accounting", "failed");
    assert!(v["stats"].is_null());
    assert!(v["stats_error"]
        .as_str()
        .unwrap()
        .contains("trellis_commits"));
}

fn seed_replay_capsule(state: &std::sync::Arc<DaemonState>) {
    let mut script = DataTransactionScript::new();
    script
        .step("tick")
        .operation(InputFact::StatusDrive(StatusDrive::Tick {
            pubkey: "missing".into(),
            at: 1_700_000_010,
        }))
        .commit();
    let script_json = script.to_json().unwrap();
    state.with_store(|s| {
        s.record_replay_capsule(&crate::state::trellis_replay_capsules::NewReplayCapsule {
            surface: "status".into(),
            trigger_kind: "tick".into(),
            trigger_ref: "missing".into(),
            script_json,
            format_version: 1,
            created_at: 1_700_000_010,
        })
        .unwrap();
    });
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
