use super::*;

/// `channel_create` (the launch channel picker's "create new channel" path)
/// must auto-create the parent channel group when it doesn't exist on the relay
/// yet. With per-session rooms off (the default), the picker can be the FIRST
/// thing to touch a channel, so the parent isn't guaranteed to exist; without
/// the parent-ensure the relay rejects the 9007 with "parent group doesn't
/// exist". Regression for that path.
#[test]
fn channel_create_auto_creates_missing_parent_channel() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let relay = shared_nip29_relay_url();
    // A fresh parent channel that has NEVER been opened on the relay.
    let parent = unique_session("freshproj");
    let backend_pk = pubkey_of(EXAMPLE_BACKEND_SEC_HEX);

    let (child_h, sibling_h) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let first = c
            .call(
                "channel_create",
                serde_json::json!({
                    "parent": parent,
                    "name": "tester",
                    "about": "tester",
                    "agents": [{ "slug": "coder", "backend": "test-host" }],
                }),
            )
            .await
            .expect("channel_create should succeed even when the parent is new");
        let second = c
            .call(
                "channel_create",
                serde_json::json!({
                    "parent": parent,
                    "name": "reviewer",
                    "about": "reviewer",
                    "agents": [],
                }),
            )
            .await
            .expect("a sibling channel should preserve the first relationship");
        (
            first["child_h"]
                .as_str()
                .expect("first child_h returned")
                .to_string(),
            second["child_h"]
                .as_str()
                .expect("second child_h returned")
                .to_string(),
        )
    });

    assert!(!child_h.is_empty(), "channel_create returned a child id");
    assert!(
        !sibling_h.is_empty(),
        "channel_create returned a sibling id"
    );

    let parent_metadata = fetch_group_metadata(&relay, &parent);
    assert!(
        has_metadata_tag(&parent_metadata, "child", &child_h),
        "parent metadata must reciprocally confirm its first child"
    );
    assert!(
        has_metadata_tag(&parent_metadata, "child", &sibling_h),
        "adding a sibling must preserve the complete parent child set"
    );

    // The parent channel group was created + locked, so the backend management
    // key is now an admin of it. (Manageability = `is_channel_admin`; the old
    // `is_group_owned` ownership flag no longer exists.)
    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        store.is_channel_admin(&parent, &backend_pk).unwrap(),
        "parent channel {parent} should be managed (backend admin) after channel_create created it"
    );

    stop_daemon(&home);
}

fn fetch_group_metadata(relay: &str, group: &str) -> serde_json::Value {
    let output = std::process::Command::new(crate::common::nak_bin())
        .args(["req", "-k", "39000", "-d", group, relay])
        .output()
        .expect("run nak kind:39000 query");
    assert!(
        output.status.success(),
        "nak metadata query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .max_by_key(|event| event["created_at"].as_u64().unwrap_or_default())
        .expect("relay returned kind:39000 metadata")
}

fn has_metadata_tag(event: &serde_json::Value, name: &str, value: &str) -> bool {
    event["tags"].as_array().is_some_and(|tags| {
        tags.iter().any(|tag| {
            tag.as_array().is_some_and(|parts| {
                parts.first().and_then(serde_json::Value::as_str) == Some(name)
                    && parts.get(1).and_then(serde_json::Value::as_str) == Some(value)
            })
        })
    })
}

/// `channel create` run as an agent (harness_session set) with NO `--agent` targets
/// nests the new channel under the creator's CURRENT channel and auto-switches the
/// running session into it. One test covers three behaviors: `--agent` is optional,
/// the parent defaults to the current channel, and the creator auto-switches.
#[test]
fn channel_create_no_agents_nests_under_current_and_auto_switches() {
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
            hook_session_start(serde_json::json!({"agent": "coder", "harness_session": sid, "cwd": "/tmp", "channel": parent, "watch_pid": std::process::id()}), "claude-code"),
        )
        .await
        .expect("session_start");
    });
    let lookup_store = Store::open(&home.store_path()).unwrap();
    let pubkey = pubkey_for_harness_session(&lookup_store, "claude-code", &sid).unwrap();
    let current_channel = lookup_store
        .get_session(&pubkey)
        .unwrap()
        .unwrap()
        .channel_h;

    // Create a child channel as that agent with NO agents and no explicit parent.
    let v = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channel_create",
            serde_json::json!({
                "name": "subtask",
                "agents": [],
                "harness_session": sid,
                "harness": "claude-code",
                "agent": "coder",
                "cwd": "/tmp",
            }),
        )
        .await
        .expect("channel_create with no agents should succeed")
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
    let rec = store.get_session(&pubkey).unwrap().expect("session row");
    assert_eq!(
        rec.channel_h, child_h,
        "session route scope should follow the auto-switch onto the new channel"
    );

    stop_daemon(&home);
}

/// Channel names are unique per parent: re-running `channel create` with a name
/// that already exists under the same parent is a hard ERROR (not a silent dedup),
/// so the agent learns the channel is already there and switches in instead.
#[test]
fn channel_create_errors_when_name_already_exists() {
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
        c.call("channel_create", mk())
            .await
            .expect("first create of a fresh name succeeds");
        let err = c
            .call("channel_create", mk())
            .await
            .expect_err("re-creating the same name under the same parent must error");
        assert!(
            format!("{err:?}").contains("already exists"),
            "error must tell the agent the channel already exists, got: {err:?}"
        );
    });

    stop_daemon(&home);
}

#[test]
fn channel_create_rejects_workspace_self_nesting() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let parent = unique_session("workspace-root");

    let error = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_create",
                serde_json::json!({
                    "parent": parent,
                    "name": parent,
                    "agents": [],
                }),
            )
            .await
            .expect_err("workspace root cannot be created beneath itself")
    });
    assert!(
        format!("{error:#}").contains("workspace root channel"),
        "unexpected error: {error:#}"
    );

    stop_daemon(&home);
}
