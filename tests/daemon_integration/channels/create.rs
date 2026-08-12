use super::*;

#[path = "create/admin_inheritance.rs"]
mod admin_inheritance;

fn start_creator(home: &Home, sid: &str) -> String {
    initialize_workspace_root("tmp", "/tmp");
    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "coder",
                        "harness_session": sid,
                        "cwd": "/tmp",
                        "channel": "tmp",
                        "watch_pid": std::process::id(),
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("session_start");
    });
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            observed_channel_members("#tmp").is_some()
        }),
        "NMP did not deliver #tmp before creator startup completed"
    );
    let store = Store::open(&home.store_path()).unwrap();
    pubkey_for_harness_session(&store, "claude-code", sid).expect("creator pubkey")
}

fn named_child_h(_home: &Home, parent_h: &str, name: &str) -> String {
    observed_channel_h(parent_h, name)
        .unwrap_or_else(|| panic!("missing NMP-observed child {name:?} beneath {parent_h:?}"))
}

/// Creating siblings through an absolute parent path returns their public
/// paths, preserves the complete parent relationship, and additively joins the
/// calling session to both.
#[test]
fn channel_create_returns_public_paths_and_preserves_siblings() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let relay = shared_nip29_relay_url();
    let parent = "tmp";
    let sid = unique_session("freshproj-creator");
    let creator = start_creator(&home, &sid);
    let first_name = unique_session("tester");
    let second_name = unique_session("reviewer");
    let backend_pk = pubkey_of(EXAMPLE_BACKEND_SEC_HEX);
    let routes_before = session_routes(&Store::open(&home.store_path()).unwrap(), &creator);

    let (first, second) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let first = c
            .call(
                "channel_create",
                serde_json::json!({
                    "channel": format!("#tmp/{first_name}"),
                    "about": "tester",
                    "agents": [{ "slug": "coder", "backend": "test-host" }],
                    "session": &creator,
                }),
            )
            .await
            .expect("create first child");
        let second = c
            .call(
                "channel_create",
                serde_json::json!({
                    "channel": format!("#tmp/{second_name}"),
                    "about": "reviewer",
                    "agents": [],
                    "session": &creator,
                }),
            )
            .await
            .expect("a sibling channel should preserve the first relationship");
        (first, second)
    });

    assert_eq!(first["channel"], format!("#tmp/{first_name}"));
    assert_eq!(second["channel"], format!("#tmp/{second_name}"));
    assert_eq!(first["joined"].as_bool(), Some(true));
    assert_eq!(second["joined"].as_bool(), Some(true));

    let child_h = named_child_h(&home, parent, &first_name);
    let sibling_h = named_child_h(&home, parent, &second_name);

    let parent_metadata = fetch_group_metadata(&relay, parent);
    assert!(
        has_metadata_tag(&parent_metadata, "child", &child_h),
        "parent metadata must reciprocally confirm its first child"
    );
    assert!(
        has_metadata_tag(&parent_metadata, "child", &sibling_h),
        "adding a sibling must preserve the complete parent child set"
    );

    // The parent channel group was created + locked, so NMP's delivered roster
    // names the backend management key as an administrator.
    let store = Store::open(&home.store_path()).unwrap();
    let routes_after = session_routes(&store, &creator);
    assert!(routes_before
        .iter()
        .all(|route| routes_after.contains(route)));
    assert!(routes_after.contains(&child_h));
    assert!(routes_after.contains(&sibling_h));
    assert!(
        observed_channel_has_role("#tmp", &backend_pk, "admin"),
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

/// An agent can create an empty channel at an explicit public path. Creation
/// joins that channel without replacing any channel the session already joined.
#[test]
fn channel_create_no_agents_adds_join_without_replacing_routes() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-create");
    let parent = "tmp";
    let pubkey = start_creator(&home, &sid);
    let child_name = unique_session("subtask");
    let lookup_store = Store::open(&home.store_path()).unwrap();
    let routes_before = session_routes(&lookup_store, &pubkey);

    // Create a child channel as that agent with no orchestration targets.
    let v = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channel_create",
            serde_json::json!({
                "channel": format!("#tmp/{child_name}"),
                "agents": [],
                "session": &pubkey,
            }),
        )
        .await
        .expect("channel_create with no agents should succeed")
    });

    let child_path = format!("#tmp/{child_name}");
    assert_eq!(v["channel"], child_path);
    assert_eq!(v["joined"].as_bool(), Some(true));
    assert_eq!(
        v["orchestration_event_id"].as_str().unwrap_or("<missing>"),
        "",
        "no --agent targets -> no kind:9 orchestration event"
    );

    let store = Store::open(&home.store_path()).unwrap();
    let child_h = named_child_h(&home, parent, &child_name);
    assert_eq!(
        observed_channel_h(parent, &child_name).as_deref(),
        Some(child_h.as_str()),
        "NMP's delivered topology should resolve the child beneath its explicit parent"
    );
    let routes_after = session_routes(&store, &pubkey);
    assert!(routes_before
        .iter()
        .all(|route| routes_after.contains(route)));
    assert!(routes_after.contains(&child_h));

    stop_daemon(&home);
}

/// Channel names are unique per parent: re-running `channel create` with a name
/// that already exists under the same parent is a hard ERROR (not a silent dedup),
/// so the agent learns the channel already exists and can join it explicitly.
#[test]
fn channel_create_errors_when_name_already_exists() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("dup-creator");
    let creator = start_creator(&home, &sid);
    let name = unique_session("dup");
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let mk = || {
            serde_json::json!({
                "channel": format!("#tmp/{name}"),
                "agents": [{ "slug": "coder", "backend": "test-host" }],
                "session": &creator,
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
    let parent = "tmp";
    let sid = unique_session("self-nesting-creator");
    let creator = start_creator(&home, &sid);

    let error = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_create",
                serde_json::json!({
                    "channel": format!("#tmp/{parent}"),
                    "agents": [],
                    "session": creator,
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
