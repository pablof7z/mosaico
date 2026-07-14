use super::*;

/// An orchestration-spawned session (the backend set `TENEX_EDGE_CHANNEL` to add
/// this agent to a task subgroup) joins that group as-is and does NOT mint a
/// child room. Guards the discriminator boundary.
#[test]
fn orchestration_session_uses_existing_group_without_minting() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "harness_session": "sess-orch-1", "cwd": "/tmp", "channel": "issue-42"}),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session(&pubkey_for_harness_session(&store, "claude-code", "sess-orch-1").unwrap())
        .unwrap()
        .expect("session row");
    let mut channel = None;
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            channel = Store::open(&home.store_path())
                .and_then(|store| store.get_channel(&rec.channel_h))
                .unwrap_or(None);
            // The channel row can materialize (from the 39002 members snapshot)
            // BEFORE its 39000 metadata (name/parent) arrives, so wait for the
            // full metadata rather than mere row existence — otherwise the
            // name/parent asserts below race the relay.
            channel
                .as_ref()
                .map(|c| c.name == "issue-42" && c.parent == "tmp")
                .unwrap_or(false)
        }),
        "channel row {} did not materialize with name+parent; got={:?}; daemon_log={}",
        rec.channel_h,
        channel,
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<{e}>"))
    );
    let channel = channel.unwrap();
    assert_eq!(channel.name, "issue-42");
    assert_eq!(
        channel.parent, "tmp",
        "with a channel override the session joins the named task channel under the channel root; it must not mint a per-session room"
    );

    stop_daemon(&home);
}
