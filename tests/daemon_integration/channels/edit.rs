use super::*;

fn find_channel_about(value: &serde_json::Value, path: &str) -> Option<String> {
    if value.get("path").and_then(serde_json::Value::as_str) == Some(path) {
        return value
            .get("about")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }
    value
        .as_array()
        .and_then(|values| {
            values
                .iter()
                .find_map(|value| find_channel_about(value, path))
        })
        .or_else(|| {
            value.as_object().and_then(|fields| {
                fields
                    .values()
                    .find_map(|value| find_channel_about(value, path))
            })
        })
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

    let child_path = format!("#tmp/{child_name}");
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
                "channel": format!("#tmp/{child_name}"),
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

    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            mosaico::daemon::blocking::call(
                "channel_list",
                serde_json::json!({ "recursive": true }),
            )
            .ok()
            .and_then(|list| find_channel_about(&list, &child_path))
            .as_deref()
                == Some("new about")
        }),
        "NMP's delivered channel view did not reach the edited about"
    );

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

    initialize_workspace_root(&root, "/tmp");
    let creator = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let started = c
            .call(
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
        started["pubkey"]
            .as_str()
            .expect("session pubkey")
            .to_string()
    });

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        for path in [
            format!("#{root}/planning"),
            format!("#{root}/epic"),
            format!("#{root}/epic/planning"),
        ] {
            let created = c
                .call(
                    "channel_create",
                    serde_json::json!({
                        "channel": path,
                        "agents": [],
                        "session": &creator,
                    }),
                )
                .await
                .unwrap_or_else(|error| panic!("create {path}: {error:#}"));
            assert_eq!(created["channel"], path);
        }
    });

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
        format!("#{root}/planning"),
        format!("#{root}/epic/planning"),
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
