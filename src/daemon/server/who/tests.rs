use super::*;
use crate::state::RegisterSession;

#[tokio::test]
async fn rejects_agent_hints_and_live_exact_session_anchors() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|s| {
        s.register_session(&RegisterSession {
            harness: "codex".into(),
            external_id_kind: "pty_session".into(),
            external_id: "pty-1".into(),
            agent_pubkey: "pk".into(),
            agent_slug: "codex".into(),
            channel_h: "root".into(),
            child_pid: Some(42),
            transcript_path: None,
            resume_id: String::new(),
            now: 1,
        })
        .unwrap();
    });
    for params in [
        serde_json::json!({ "agent": "codex" }),
        serde_json::json!({ "group": "root" }),
        serde_json::json!({ "pty_session": "pty-1" }),
    ] {
        let err = rpc_who(&state, &params).expect_err("agent who must be rejected");
        assert!(
            err.to_string()
                .contains("agents use `tenex-edge my session`"),
            "{err:#}"
        );
    }
}

#[tokio::test]
async fn stale_unresolved_process_anchor_does_not_turn_an_operator_into_an_agent() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|s| s.upsert_channel("root", "root", "", "", 1).unwrap());

    let out = rpc_who(
        &state,
        &serde_json::json!({
            "workspace": "root",
            "watch_pid": 999_999,
            "human_color": false
        }),
    )
    .unwrap();

    assert!(out.get("fabric_human").is_some());
}

#[tokio::test]
async fn human_who_never_returns_agent_fabric() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|s| s.upsert_channel("root", "root", "", "", 1).unwrap());

    let out = rpc_who(
        &state,
        &serde_json::json!({ "workspace": "root", "human_color": false }),
    )
    .unwrap();

    assert!(out.get("fabric_human").is_some());
    assert!(out.get("fabric").is_none());
}
