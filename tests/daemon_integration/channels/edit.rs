use super::*;

#[test]
fn channels_edit_updates_about_from_relay_truth() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-edit");
    let parent = unique_session("edit-parent");
    let watch_pid = std::process::id() as i32;

    let child_h = rt().block_on(async {
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

        let created = c
            .call(
                "channels_create",
                serde_json::json!({
                    "name": "editable",
                    "about": "old about",
                    "agents": [],
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("channels_create");
        created["child_h"].as_str().unwrap().to_string()
    });

    let edited = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channels_edit",
            serde_json::json!({
                "channel": "editable",
                "about": "new about",
                "harness": "claude-code",
                "watch_pid": watch_pid,
                "agent": "coder",
                "cwd": "/tmp"
            }),
        )
        .await
        .expect("channels_edit")
    });

    assert_eq!(edited["channel"].as_str(), Some(child_h.as_str()));
    assert_eq!(edited["about"].as_str(), Some("new about"));
    assert_eq!(edited["confirmed"].as_bool(), Some(true));

    let store = Store::open(&home.store_path()).unwrap();
    let channel = store.get_channel(&child_h).unwrap().expect("channel row");
    assert_eq!(channel.about, "new about");

    stop_daemon(&home);
}

#[test]
fn channels_edit_ambiguous_reference_returns_exact_reruns() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-edit-ambiguous");
    let root = unique_session("edit-root");
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
                "channel": &root,
                "watch_pid": watch_pid
            }),
        )
        .await
        .expect("session_start");
    });

    let active_channel = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sid)
        .unwrap()
        .expect("session row")
        .channel_h;
    let actual_root = Store::open(&home.store_path())
        .unwrap()
        .root_channel_of(&active_channel)
        .unwrap()
        .unwrap_or(active_channel);
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("h-direct", "planning", "", &actual_root, 1)
        .unwrap();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("h-epic", "epic", "", &actual_root, 1)
        .unwrap();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("h-nested", "planning", "", "h-epic", 1)
        .unwrap();

    let v = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channels_edit",
            serde_json::json!({
                "channel": "planning",
                "about": "new about",
                "harness": "claude-code",
                "watch_pid": watch_pid,
                "agent": "coder",
                "cwd": "/tmp"
            }),
        )
        .await
        .expect("ambiguous edit returns structured reruns")
    });

    let refs = v["ambiguous"].as_array().expect("ambiguous refs");
    assert_eq!(refs.len(), 2);
    assert!(refs.iter().any(|v| v.as_str() == Some("planning")));
    assert!(refs.iter().any(|v| v.as_str() == Some("epic/planning")));

    stop_daemon(&home);
}
