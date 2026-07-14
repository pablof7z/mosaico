use super::*;
use crate::reconcile::{CursorSeed, TurnProjectionSeed};
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_turn_target_checks_local_projection() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1");
    seed_turn_graph(&state, "s1", true, 100, Some("tx1"));
    state
        .with_store(|s| s.apply_turn_projection("s1", true, 100, Some("tx1")))
        .unwrap();

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "turn:s1" })).unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "turn_outcome", "passed");
    assert_eq!(v["turn_evidence"]["working_matches_session"], true);
    assert_eq!(v["turn_evidence"]["started_matches_session"], true);
    assert_eq!(v["turn_evidence"]["transcript_matches_session"], true);
}

#[tokio::test]
async fn rpc_probe_validate_turn_target_fails_projection_mismatch() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1");
    seed_turn_graph(&state, "s1", true, 100, None);

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "turn:s1" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "turn_outcome", "failed");
    assert_eq!(v["turn_evidence"]["working_matches_session"], false);
    assert!(v["turn_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("does not match"));
}

#[tokio::test]
async fn rpc_probe_validate_cursor_target_checks_local_projection() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1");
    seed_cursor_graph(&state, "s1", 10, 25);
    state
        .with_store(|s| s.apply_cursor_projection("s1", 25))
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "cursor:s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "cursor_outcome", "passed");
    assert_eq!(v["cursor_evidence"]["cursor_matches_session"], true);
}

#[tokio::test]
async fn rpc_probe_validate_cursor_target_fails_projection_mismatch() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1");
    seed_cursor_graph(&state, "s1", 10, 25);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "cursor:s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "cursor_outcome", "failed");
    assert_eq!(v["cursor_evidence"]["cursor_matches_session"], false);
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

fn seed_turn_graph(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    working: bool,
    turn_started_at: u64,
    transcript_ref: Option<&str>,
) {
    state
        .turn_lifecycle
        .lock()
        .unwrap()
        .on_turn_started(
            TurnProjectionSeed {
                pubkey: pubkey.into(),
                working: false,
                turn_started_at: 0,
                transcript_ref: None,
            },
            turn_started_at,
            transcript_ref.map(str::to_string),
        )
        .unwrap();
    if !working {
        state
            .turn_lifecycle
            .lock()
            .unwrap()
            .on_turn_ended(
                TurnProjectionSeed {
                    pubkey: pubkey.into(),
                    working: true,
                    turn_started_at,
                    transcript_ref: transcript_ref.map(str::to_string),
                },
                turn_started_at + 1,
            )
            .unwrap();
    }
}

fn seed_cursor_graph(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    seen_cursor: u64,
    next_cursor: u64,
) {
    state
        .cursor
        .lock()
        .unwrap()
        .request(
            CursorSeed {
                pubkey: pubkey.into(),
                seen_cursor,
            },
            InputFact::TurnCheckRequested {
                pubkey: pubkey.into(),
                observed_cursor: seen_cursor,
                working: true,
                at: next_cursor,
            },
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
