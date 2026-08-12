use super::*;
#[tokio::test]
async fn agent_context_does_not_block_explicit_who() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|store| {
        store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
            crate::state::TestGroup::new("root").metadata("root", "", "", 1),
        ]));
    });

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
    state.with_store(|store| {
        store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
            crate::state::TestGroup::new("root").metadata("root", "", "", 1),
        ]));
    });

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
    state.with_store(|store| {
        store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
            crate::state::TestGroup::new("root").metadata("root", "", "", 1),
        ]));
    });

    let out = rpc_who(
        &state,
        &serde_json::json!({ "workspace": "root", "human_color": false }),
    )
    .unwrap();

    assert!(out.get("fabric_human").is_some());
    assert!(out.get("fabric").is_none());
}

#[tokio::test]
async fn raw_who_json_uses_public_channel_paths() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|store| {
        store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
            crate::state::TestGroup::new("root-h").metadata("root", "", "", 1),
            crate::state::TestGroup::new("child-h").metadata("review", "", "root-h", 2),
        ]));
        store.install_test_nmp_relay_delivery(crate::state::TestRelayDelivery::new().profiles([
            crate::state::Profile {
                pubkey: "agent-pk".into(),
                name: "reviewer".into(),
                slug: "reviewer".into(),
                agent_slug: "reviewer".into(),
                host: "test-host".into(),
                is_backend: false,
                agents: Vec::new(),
                workspaces: Vec::new(),
                updated_at: 3,
            },
        ]));
        store
            .reserve_hook_session_for_test(&crate::state::RegisterSession {
                pubkey: "agent-pk".into(),
                observed_harness: "codex".into(),
                agent_slug: "reviewer".into(),
                launch_channel_h: "child-h".into(),
                work_root: "root-h".into(),
                child_pid: None,
                now: 3,
            })
            .unwrap();
    });

    let out = rpc_who(
        &state,
        &serde_json::json!({ "channel": "#root-h/review", "human_color": false }),
    )
    .unwrap();
    assert_eq!(out["root"], "#root-h/review");
    assert_eq!(out["rows"][0]["channel"], "#root-h/review");
    assert_eq!(out["rows"][0]["work_root"], "#root-h");
    assert!(!out.to_string().contains("\"child-h\""), "{out:#}");
}
