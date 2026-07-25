use super::*;

#[test]
fn channel_edit_updates_about_from_relay_truth() {
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

        let created = c
            .call(
                "channel_create",
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
            .expect("channel_create");
        created["child_h"].as_str().unwrap().to_string()
    });

    let edited = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channel_edit",
            serde_json::json!({
                "channel": format!("@{child_h}"),
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

    assert_eq!(edited["channel"].as_str(), Some(child_h.as_str()));
    assert_eq!(edited["about"].as_str(), Some("new about"));
    assert_eq!(edited["confirmed"].as_bool(), Some(true));

    let store = Store::open(&home.store_path()).unwrap();
    let channel = store.get_channel(&child_h).unwrap().expect("channel row");
    assert_eq!(channel.about, "new about");

    stop_daemon(&home);
}

/// Under full-path addressing a bare name is not a reference at all, and a
/// well-formed absolute path names exactly one channel. Only an `@<id-prefix>`
/// can still be ambiguous — and its reruns are exact opaque-id selectors.
#[test]
fn channel_edit_rejects_bare_names_and_disambiguates_id_prefixes() {
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
    let active_channel = session_for_harness_session(&store, "claude-code", &sid).channel_h;
    let actual_root = Store::open(&home.store_path())
        .unwrap()
        .root_channel_of(&active_channel)
        .unwrap()
        .unwrap_or(active_channel);
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel(&actual_root, &actual_root, "", "", 1)
        .unwrap();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("h-plan-direct", "planning", "", &actual_root, 1)
        .unwrap();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("h-epic", "epic", "", &actual_root, 1)
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

    // An `@<id-prefix>` matching several channels returns exact `@id` reruns.
    let v = edit("@h-plan-".to_string()).expect("ambiguous edit returns structured reruns");
    let refs = v["ambiguous"].as_array().expect("ambiguous refs");
    assert_eq!(refs.len(), 2, "{v}");
    assert!(refs.iter().any(|v| v.as_str() == Some("@h-plan-direct")));
    assert!(refs.iter().any(|v| v.as_str() == Some("@h-plan-nested")));

    // Both same-named channels stay individually addressable by full path, and
    // neither path is ever ambiguous.
    for path in [
        format!("/{actual_root}/planning"),
        format!("/{actual_root}/epic/planning"),
    ] {
        let rendered = match edit(path.clone()) {
            Ok(v) => v.to_string(),
            Err(e) => format!("{e:#}"),
        };
        assert!(
            !rendered.contains("ambiguous"),
            "{path} must name exactly one channel: {rendered}"
        );
    }

    stop_daemon(&home);
}
