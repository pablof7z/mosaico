use super::*;

fn named_child_h(home: &Home, parent_h: &str, name: &str) -> String {
    Store::open(&home.store_path())
        .unwrap()
        .channel_id_for_name(parent_h, name)
        .unwrap()
        .unwrap_or_else(|| panic!("missing child {name:?} beneath {parent_h:?}"))
}

#[test]
fn channel_create_uses_watch_pid_as_exact_session_anchor() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-watch-create");
    let parent = "tmp".to_string();
    let watch_pid = std::process::id() as i32;

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "coder",
                    "harness_session": &sid,
                    "cwd": "/tmp",
                    "channel": &parent,
                    "watch_pid": watch_pid
                }),
                "claude-code",
            ),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let session = session_for_harness_session(&store, "claude-code", &sid);
    let routes_before = session_routes(&store, &session.pubkey);

    let created = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channel_create",
            serde_json::json!({
                "channel": format!("#{parent}/native-subtask"),
                "agents": [],
                "harness": "claude-code",
                "watch_pid": watch_pid,
                "agent": "coder",
                "cwd": "/tmp"
            }),
        )
        .await
        .expect("channel_create should resolve the exact watched process")
    });
    let child_path = format!("#{parent}/native-subtask");
    assert_eq!(created["channel"], child_path);
    assert_eq!(created["joined"].as_bool(), Some(true));

    let store = Store::open(&home.store_path()).unwrap();
    let rec = session_for_harness_session(&store, "claude-code", &sid);
    let child_h = named_child_h(&home, &parent, "native-subtask");
    let routes_after = session_routes(&store, &rec.pubkey);
    assert!(routes_before
        .iter()
        .all(|route| routes_after.contains(route)));
    assert!(routes_after.contains(&child_h));
    assert_eq!(
        store.channel_parent(&child_h).unwrap().unwrap_or_default(),
        parent,
        "new channel should nest under the explicit public parent"
    );

    stop_daemon(&home);
}

#[test]
fn explicit_who_and_my_session_accept_the_exact_anchor() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-watch-who");
    let parent = unique_session("who-parent");
    let watch_pid = std::process::id() as i32;

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "coder",
                    "harness_session": &sid,
                    "cwd": "/tmp",
                    "channel": &parent,
                    "watch_pid": watch_pid
                }),
                "claude-code",
            ),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let session = session_for_harness_session(&store, "claude-code", &sid);
    let joined_channel_h = only_session_route(&store, &session.pubkey);
    store
        .upsert_channel(&joined_channel_h, "who-parent", "", "", 1)
        .unwrap();
    store
        .replace_channel_members(&joined_channel_h, &[session.pubkey], 1)
        .unwrap();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let who = c
            .call("who", serde_json::json!({"agent": "coder", "cwd": "/tmp"}))
            .await
            .expect("explicit agent who should remain available");
        assert!(
            who["fabric_human"].as_str().is_some(),
            "who should return the read-only fabric view: {who:#}"
        );

        let who = c
            .call(
                "who",
                serde_json::json!({
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("agent-anchored who should remain available");
        assert!(
            who["fabric_human"].as_str().is_some(),
            "who should return the read-only fabric view: {who:#}"
        );

        let briefing = c
            .call(
                "my_session",
                serde_json::json!({
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("my session should accept the exact watched-process anchor");
        let fabric = briefing["fabric"].as_str().expect("agent briefing");
        assert!(fabric.contains("<mosaico>"), "got: {fabric}");
        assert!(
            fabric.contains(&format!("name=\"/{parent}\"")),
            "got: {fabric}"
        );
    });

    stop_daemon(&home);
}

#[test]
fn channel_membership_commands_use_watch_pid_as_exact_session_anchor() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-watch-membership");
    let parent = "tmp".to_string();
    let watch_pid = std::process::id() as i32;

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "coder",
                    "harness_session": &sid,
                    "cwd": "/tmp",
                    "channel": &parent,
                    "watch_pid": watch_pid
                }),
                "claude-code",
            ),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let pubkey = pubkey_for_harness_session(&store, "claude-code", &sid).unwrap();
    let routes_before = session_routes(&store, &pubkey);

    let created = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channel_create",
            serde_json::json!({
                "channel": format!("#{parent}/membership-child"),
                "agents": [],
                "harness": "claude-code",
                "watch_pid": watch_pid,
                "agent": "coder",
                "cwd": "/tmp"
            }),
        )
        .await
        .expect("create should resolve by watched process")
    });
    let child_path = format!("#{parent}/membership-child");
    assert_eq!(created["channel"], child_path);
    assert_eq!(created["joined"].as_bool(), Some(true));
    let child_h = named_child_h(&home, &parent, "membership-child");

    // Leave and rejoin the workspace root through its public path. The child
    // route remains independent throughout.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");

        let left = c
            .call(
                "channel_leave",
                serde_json::json!({
                    "channel": format!("#{parent}"),
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("leave should resolve by watched process");
        assert_eq!(left["channel"], format!("#{parent}"));
        assert_eq!(left["left"].as_bool(), Some(true));

        let joined = c
            .call(
                "channel_join",
                serde_json::json!({
                    "channel": format!("#{parent}"),
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("join should resolve by watched process");
        assert_eq!(joined["channel"], format!("#{parent}"));
    });

    let store = Store::open(&home.store_path()).unwrap();
    let routes_after = session_routes(&store, &pubkey);
    assert!(routes_before
        .iter()
        .all(|route| routes_after.contains(route)));
    assert!(routes_after.contains(&child_h));
    assert!(store
        .has_session_route(&pubkey, &parent)
        .expect("joined-channel check"));

    stop_daemon(&home);
}
