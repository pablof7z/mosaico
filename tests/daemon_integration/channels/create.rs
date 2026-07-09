use super::*;

/// `channels_create` (the launch channel picker's "create new channel" path)
/// must auto-create the parent channel group when it doesn't exist on the relay
/// yet. With per-session rooms off (the default), the picker can be the FIRST
/// thing to touch a channel, so the parent isn't guaranteed to exist; without
/// the parent-ensure the relay rejects the 9007 with "parent group doesn't
/// exist". Regression for that path.
#[test]
fn channels_create_auto_creates_missing_parent_channel() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    // A fresh parent channel that has NEVER been opened on the relay.
    let parent = unique_session("freshproj");
    let backend_pk = pubkey_of(EXAMPLE_BACKEND_SEC_HEX);

    let child_h = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "channels_create",
                serde_json::json!({
                    "parent": parent,
                    "name": "tester",
                    "about": "tester",
                    "agents": [{ "slug": "coder", "backend": "test-host" }],
                }),
            )
            .await
            .expect("channels_create should succeed even when the parent is new");
        v["child_h"].as_str().expect("child_h returned").to_string()
    });

    assert!(!child_h.is_empty(), "channels_create returned a child id");

    // The parent channel group was created + locked, so the backend management
    // key is now an admin of it. (Manageability = `is_channel_admin`; the old
    // `is_group_owned` ownership flag no longer exists.)
    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        store.is_channel_admin(&parent, &backend_pk).unwrap(),
        "parent channel {parent} should be managed (backend admin) after channels_create created it"
    );

    stop_daemon(&home);
}

/// `channels create` run as an agent (env_session set) with NO `--agent` targets
/// nests the new channel under the creator's CURRENT channel and auto-switches the
/// running session into it. One test covers three behaviors: `--agent` is optional,
/// the parent defaults to the current channel, and the creator auto-switches.
#[test]
fn channels_create_no_agents_nests_under_current_and_auto_switches() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-create");
    let parent = unique_session("currentchan");

    // Start a session pinned to a known current channel (the override wins over
    // any per-session room), kept alive by watching this test process. The channel
    // NAME resolves to an opaque id, so read back the session's actual `channel_h`.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": sid, "cwd": "/tmp", "channel": parent, "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });
    let current_channel = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sid)
        .unwrap()
        .expect("session row")
        .channel_h;

    // Create a child channel as that agent with NO agents and no explicit parent.
    let v = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channels_create",
            serde_json::json!({
                "name": "subtask",
                "agents": [],
                "env_session": sid,
                "agent": "coder",
                "cwd": "/tmp",
            }),
        )
        .await
        .expect("channels_create with no agents should succeed")
    });

    let child_h = v["child_h"].as_str().expect("child_h returned").to_string();
    assert!(
        v["switched"].as_bool().unwrap_or(false),
        "the creating session should auto-switch into the new channel"
    );
    assert_eq!(
        v["orchestration_event_id"].as_str().unwrap_or("<missing>"),
        "",
        "no --agent targets -> no kind:9 orchestration event"
    );

    let store = Store::open(&home.store_path()).unwrap();
    // The new channel nests under the creator's CURRENT channel, not the channel root.
    assert_eq!(
        store.channel_parent(&child_h).unwrap().unwrap_or_default(),
        current_channel,
        "new channel should nest under the creator's current channel"
    );
    // The creating session is re-homed onto the new channel.
    let rec = store.get_session(&sid).unwrap().expect("session row");
    assert_eq!(
        rec.channel_h, child_h,
        "session route scope should follow the auto-switch onto the new channel"
    );

    stop_daemon(&home);
}

/// Channel names are unique per parent: re-running `channels create` with a name
/// that already exists under the same parent is a hard ERROR (not a silent dedup),
/// so the agent learns the channel is already there and switches in instead.
#[test]
fn channels_create_errors_when_name_already_exists() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let parent = unique_session("dupproj");
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let mk = || {
            serde_json::json!({
                "parent": parent,
                "name": "dup",
                "agents": [{ "slug": "coder", "backend": "test-host" }],
            })
        };
        c.call("channels_create", mk())
            .await
            .expect("first create of a fresh name succeeds");
        let err = c
            .call("channels_create", mk())
            .await
            .expect_err("re-creating the same name under the same parent must error");
        assert!(
            format!("{err:?}").contains("already exists"),
            "error must tell the agent the channel already exists, got: {err:?}"
        );
    });

    stop_daemon(&home);
}
