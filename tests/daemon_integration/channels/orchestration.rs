use super::*;

/// An orchestration-spawned session joins the requested task channel as-is and
/// does not mint a child. Guards the launch-channel discriminator boundary.
#[test]
fn orchestration_session_uses_existing_group_without_minting() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    initialize_workspace_root("tmp", "/tmp");
    wait_for_channel_metadata(&home, "tmp");

    let creator = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let creator = c
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "creator",
                        "harness_session": "sess-orch-creator",
                        "cwd": "/tmp",
                        "channel": "tmp"
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("start creator")["pubkey"]
            .as_str()
            .unwrap()
            .to_string();
        c.call(
            "channel_create",
            serde_json::json!({
                "channel": "#tmp/issue-42",
                "agents": [],
                "session": &creator,
            }),
        )
        .await
        .expect("create the existing orchestration group");
        creator
    });
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            observed_channel_has_role("#tmp", &creator, "member")
                && observed_channel_h("tmp", "issue-42").is_some()
        }),
        "creator or existing task channel was not delivered through NMP"
    );
    let task_h = observed_channel_h("tmp", "issue-42").unwrap();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "coder",
                    "harness_session": "sess-orch-1",
                    "cwd": "/tmp",
                    "channel": &task_h
                }),
                "claude-code",
            ),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session(&pubkey_for_harness_session(&store, "claude-code", "sess-orch-1").unwrap())
        .unwrap()
        .expect("session row");
    let channel_h = only_session_route(&store, &rec.pubkey);
    assert_eq!(channel_h, task_h, "the existing task group must be reused");
    let mut observed = None;
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            observed = observed_channel_h("tmp", "issue-42");
            observed.as_deref() == Some(channel_h.as_str())
                && observed_channel_members("#tmp/issue-42").is_some_and(|members| {
                    members.iter().any(|member| {
                        member["pubkey"].as_str() == Some(rec.pubkey.as_str())
                    })
                })
        }),
        "NMP did not deliver #tmp/issue-42 with the session member; channel_h={}; observed={:?}; daemon_log={}",
        channel_h,
        observed,
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<{e}>"))
    );

    stop_daemon(&home);
}
