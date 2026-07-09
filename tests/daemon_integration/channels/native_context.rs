use super::*;

#[test]
fn channels_create_uses_watch_pid_as_exact_session_anchor() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-watch-create");
    let parent = unique_session("watch-parent");
    let watch_pid = std::process::id() as i32;

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({
                "agent": "coder",
                "session_id": &sid,
                "harness": "claude-code",
                "cwd": "/tmp",
                "channel": &parent,
                "watch_pid": watch_pid
            }),
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

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "channels_create",
                serde_json::json!({
                    "name": "native-subtask",
                    "agents": [],
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("channels_create should resolve the exact watched process");

        assert!(
            v["switched"].as_bool().unwrap_or(false),
            "watched-process caller should auto-switch"
        );
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store.get_session(&sid).unwrap().expect("session row");
    assert_ne!(
        rec.channel_h, current_channel,
        "session should have moved to the child"
    );
    assert_eq!(
        store
            .channel_parent(&rec.channel_h)
            .unwrap()
            .unwrap_or_default(),
        current_channel,
        "new channel should nest under the caller's current channel"
    );

    stop_daemon(&home);
}

#[test]
fn who_refuses_agent_channel_scan_without_exact_anchor() {
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
            serde_json::json!({
                "agent": "coder",
                "session_id": &sid,
                "harness": "claude-code",
                "cwd": "/tmp",
                "channel": &parent,
                "watch_pid": watch_pid
            }),
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

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let err = c
            .call("who", serde_json::json!({"agent": "coder", "cwd": "/tmp"}))
            .await
            .expect_err("who must not infer a caller from agent+cwd alone");
        assert!(
            format!("{err:#}").contains("exact live session anchor"),
            "unexpected who error: {err:#}"
        );

        let ok = c
            .call(
                "who",
                serde_json::json!({
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("who should accept the exact watched-process anchor");
        assert_eq!(ok["root"].as_str(), Some(current_channel.as_str()));
        assert!(
            ok.get("fabric_human").is_none(),
            "agent-anchored who should keep the XML fabric path"
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
    let parent = unique_session("membership-parent");
    let watch_pid = std::process::id() as i32;

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({
                "agent": "coder",
                "session_id": &sid,
                "harness": "claude-code",
                "cwd": "/tmp",
                "channel": &parent,
                "watch_pid": watch_pid
            }),
        )
        .await
        .expect("session_start");
    });

    let parent_h = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sid)
        .unwrap()
        .expect("session row")
        .channel_h;

    let child_h = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let created = c
            .call(
                "channels_create",
                serde_json::json!({
                    "name": "membership-child",
                    "agents": [],
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("create should resolve by watched process");
        created["child_h"].as_str().unwrap().to_string()
    });

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");

        let switched = c
            .call(
                "channels_switch",
                serde_json::json!({
                    "channel": &parent_h,
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("switch should resolve by watched process");
        assert_eq!(switched["channel"].as_str(), Some(parent_h.as_str()));
        assert_eq!(switched["prev_channel"].as_str(), Some(child_h.as_str()));

        let joined = c
            .call(
                "channels_join",
                serde_json::json!({
                    "channel": &child_h,
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("join should resolve by watched process");
        assert_eq!(joined["channel"].as_str(), Some(child_h.as_str()));
        assert_eq!(joined["active_channel"].as_str(), Some(parent_h.as_str()));

        let left = c
            .call(
                "channels_leave",
                serde_json::json!({
                    "channel": &child_h,
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("leave should resolve by watched process");
        assert_eq!(left["channel"].as_str(), Some(child_h.as_str()));
        assert_eq!(left["left"].as_bool(), Some(true));
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store.get_session(&sid).unwrap().expect("session row");
    assert_eq!(rec.channel_h, parent_h);
    assert!(
        !store
            .is_session_joined_channel(&sid, &child_h)
            .expect("joined-channel check"),
        "leave should remove the passive child-channel join"
    );

    stop_daemon(&home);
}
