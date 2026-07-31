use super::*;

fn named_child_h(home: &Home, parent_h: &str, name: &str) -> String {
    Store::open(&home.store_path())
        .unwrap()
        .channel_id_for_name(parent_h, name)
        .unwrap()
        .unwrap_or_else(|| panic!("missing child {name:?} beneath {parent_h:?}"))
}

#[test]
fn channel_edit_updates_about_from_relay_truth() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-edit");
    let parent = "tmp";
    let child_name = unique_session("editable");
    let watch_pid = std::process::id() as i32;

    let child_path = format!("/tmp/{child_name}");
    initialize_workspace_root("tmp", "/tmp");
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "coder",
                    "harness_session": &sid,
                    "cwd": "/tmp",
                    "channel": "tmp",
                    "watch_pid": watch_pid
                }),
                "claude-code",
            ),
        )
        .await
        .expect("session_start");
    });
    wait_for_channel_metadata(&home, parent);

    let created = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channel_create",
            serde_json::json!({
                "channel": format!("/tmp/{child_name}"),
                "about": "old about",
                "agents": [],
                "harness": "claude-code",
                "watch_pid": watch_pid,
                "agent": "coder",
                "cwd": "/tmp"
            }),
        )
        .await
        .expect("channel_create")
    });
    assert_eq!(created["channel"], child_path);
    assert_eq!(created["joined"].as_bool(), Some(true));
    let child_h = named_child_h(&home, parent, &child_name);

    let edited = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channel_edit",
            serde_json::json!({
                "channel": &child_path,
                "about": "new about",
                "harness": "claude-code",
                "watch_pid": watch_pid,
                "agent": "coder",
                "cwd": "/tmp"
            }),
        )
        .await
        .expect("channel_edit")
    });

    assert_eq!(edited["channel"], child_path);
    assert_eq!(edited["about"].as_str(), Some("new about"));
    assert_eq!(edited["confirmed"].as_bool(), Some(true));

    let store = Store::open(&home.store_path()).unwrap();
    let channel = store.get_channel(&child_h).unwrap().expect("channel row");
    assert_eq!(channel.about, "new about");

    stop_daemon(&home);
}

/// Under full-path addressing a bare name is not a reference at all, while
/// each well-formed absolute hierarchy names at most one channel.
#[test]
fn channel_edit_rejects_bare_names_and_resolves_full_paths() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-edit-paths");
    let root = unique_session("edit-root");
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
                    "channel": &root,
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
    let workspace_h = Store::open(&home.store_path())
        .unwrap()
        .root_channel_of(&joined_channel_h)
        .unwrap()
        .unwrap_or(joined_channel_h);
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel(&workspace_h, &workspace_h, "", "", 1)
        .unwrap();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("h-plan-direct", "planning", "", &workspace_h, 1)
        .unwrap();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("h-epic", "epic", "", &workspace_h, 1)
        .unwrap();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("h-plan-nested", "planning", "", "h-epic", 1)
        .unwrap();

    let edit = |reference: String| {
        rt().block_on(async move {
            let mut c = Client::connect_or_spawn().await.expect("connect");
            c.call(
                "channel_edit",
                serde_json::json!({
                    "channel": reference,
                    "about": "new about",
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": "/tmp"
                }),
            )
            .await
        })
    };

    // A bare name resolves nothing now — it is rejected before any lookup.
    let bare = edit("planning".to_string()).expect_err("a bare name must be rejected");
    assert!(bare.to_string().contains("must be a full path"), "{bare:#}");

    // Both same-named channels are accepted as independently addressable full
    // paths rather than being rejected as malformed references.
    for path in [
        format!("#{workspace_h}/planning"),
        format!("#{workspace_h}/epic/planning"),
    ] {
        let rendered = match edit(path.clone()) {
            Ok(v) => v.to_string(),
            Err(e) => format!("{e:#}"),
        };
        assert!(
            !rendered.contains("must be a full path"),
            "{path} must name exactly one channel: {rendered}"
        );
    }

    stop_daemon(&home);
}
