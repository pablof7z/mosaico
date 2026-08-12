use super::*;

fn map_workspace(home: &Home, channel: &str, path: &std::path::Path) {
    let map_path = home.dir.path().join("workspaces.json");
    let mut map = std::fs::read_to_string(&map_path)
        .ok()
        .and_then(|body| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&body).ok()
        })
        .unwrap_or_default();
    map.insert(
        channel.to_string(),
        path.to_string_lossy().to_string().into(),
    );
    std::fs::write(map_path, serde_json::to_vec(&map).unwrap()).unwrap();
}

fn named_child_h(home: &Home, parent_h: &str, name: &str) -> String {
    let mut child = None;
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            child = observed_channel_h(parent_h, name);
            child.is_some()
        }),
        "missing child {name:?} beneath {parent_h:?}; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    child.unwrap()
}

#[test]
fn channel_create_uses_watch_pid_as_exact_session_anchor() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-watch-create");
    let parent = "tmp".to_string();
    let watch_pid = std::process::id() as i32;
    initialize_workspace_root(&parent, "/tmp");
    wait_for_channel_metadata(&home, &parent);

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
        observed_channel_h(&parent, "native-subtask").as_deref(),
        Some(child_h.as_str()),
        "NMP's delivered topology should nest the child under the explicit public parent"
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
    let work_dir = home.dir.path().join(&parent);
    std::fs::create_dir_all(&work_dir).unwrap();
    map_workspace(&home, &parent, &work_dir);
    initialize_workspace_root(&parent, work_dir.to_str().unwrap());
    wait_for_channel_metadata(&home, &parent);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "coder",
                    "harness_session": &sid,
                    "cwd": &work_dir,
                    "channel": &parent,
                    "watch_pid": watch_pid
                }),
                "claude-code",
            ),
        )
        .await
        .expect("session_start");
    });

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let who = c
            .call(
                "who",
                serde_json::json!({"agent": "coder", "cwd": &work_dir}),
            )
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
                    "cwd": &work_dir
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
                    "cwd": &work_dir
                }),
            )
            .await
            .expect("my session should accept the exact watched-process anchor");
        let fabric = briefing["fabric"].as_str().expect("agent briefing");
        assert!(fabric.contains("<mosaico>"), "got: {fabric}");
        assert!(
            fabric.contains(&format!("name=\"#{parent}\"")),
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
    initialize_workspace_root(&parent, "/tmp");
    wait_for_channel_metadata(&home, &parent);

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
