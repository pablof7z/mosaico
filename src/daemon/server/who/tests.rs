use super::*;
#[tokio::test]
async fn agent_context_does_not_block_explicit_who() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|s| s.upsert_channel("root", "root", "", "", 1).unwrap());

    for params in [
        serde_json::json!({ "workspace": "root", "agent": "codex" }),
        serde_json::json!({ "workspace": "root", "group": "root" }),
        serde_json::json!({ "workspace": "root", "pty_session": "pty-1" }),
    ] {
        let out = rpc_who(&state, &params).expect("explicit who should remain available");
        assert!(out.get("fabric_human").is_some());
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
