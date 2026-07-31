use super::*;
use std::collections::BTreeSet;

#[test]
fn owned_mention_resumes_routeless_session_without_restoring_explicit_leaves() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let root = unique_session("kind9-routeless");
    let work_dir = home.dir.path().join(&root);
    add_workspace_mapping(&home, &root, &work_dir);
    let agent = "routeless-kind9";
    let log = home.dir.path().join("routeless-injected.log");
    let native_session = unique_session("routeless-native");
    let _path = install_opencode_shim(&home, &native_session, &work_dir, &log);
    identity::add_local_agent(home.dir.path(), agent, "offline-test", None, 1)
        .expect("add local agent");

    let (_, original) = launch_target(&home, agent, &root, &work_dir);
    assert_eq!(original.runtime_generation, 1);
    start_keeper(&home, &root, &work_dir);

    let child_name = unique_session("left-child");
    let child_path = format!("#{root}/{child_name}");
    let created = rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_create",
                serde_json::json!({
                    "channel": &child_path,
                    "agents": [],
                    "session": &original.pubkey,
                }),
            )
            .await
            .expect("create child")
    });
    assert_eq!(created["joined"], true);
    let child_h = Store::open(&home.store_path())
        .unwrap()
        .channel_id_for_name(&root, &child_name)
        .unwrap()
        .expect("child channel id");
    wait_for_group_member(&home, &child_h, &original.pubkey);

    rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        for channel in [&child_path, &format!("#{root}")] {
            let left = client
                .call(
                    "channel_leave",
                    serde_json::json!({
                        "channel": channel,
                        "session": &original.pubkey,
                    }),
                )
                .await
                .expect("explicit leave");
            assert_eq!(left["left"], true);
        }
    });

    assert!(wait_until(Duration::from_secs(15), || {
        refresh_channel_members(&format!("#{root}"));
        refresh_channel_members(&child_path);
        let store = Store::open(&home.store_path()).unwrap();
        store
            .list_session_routes(&original.pubkey)
            .unwrap()
            .is_empty()
            && !store.is_channel_member(&root, &original.pubkey).unwrap()
            && !store.is_channel_member(&child_h, &original.pubkey).unwrap()
    }));
    assert_absent(&home, &original.pubkey, &root);
    assert_absent(&home, &original.pubkey, &child_h);
    end_target(&home, &original.pubkey);

    let daemon_log_path = home.dir.path().join("daemon.log");
    let log_boundary = std::fs::read_to_string(&daemon_log_path)
        .unwrap_or_default()
        .len();
    let body = format!("deliver without rejoining {}", unique_session("body"));
    rt().block_on(publish_user_kind9(&root, &body, &original.pubkey));

    let resumed = wait_for_running_generation(&home, &original.pubkey, 2);
    assert_eq!(resumed.agent_slug, agent);
    wait_for_injected_log(&log, &body);
    assert_memberships_still_absent(&home, &original.pubkey, &root, &child_h, &child_path);

    let daemon_log = std::fs::read_to_string(&daemon_log_path).unwrap_or_default();
    let recovery_log = daemon_log.get(log_boundary..).unwrap_or(&daemon_log);
    assert!(
        !recovery_log.lines().any(|line| {
            line.contains("nip29-role-decision")
                && line.contains("role=member")
                && line.contains("reason=add member")
                && (line.contains(&format!("channel={root}"))
                    || line.contains(&format!("channel={child_h}")))
        }),
        "owned mention must not publish a member re-add after explicit leave:\n{recovery_log}"
    );

    let endpoint =
        pty_session_for_session(&Store::open(&home.store_path()).unwrap(), &original.pubkey)
            .expect("resumed endpoint");
    let cleanup = PtyProcessGuard::capture(&endpoint);
    let public_handle = Store::open(&home.store_path())
        .unwrap()
        .session_identity(&original.pubkey)
        .unwrap()
        .expect("public session identity")
        .display_slug();

    let log_boundary = daemon_log_boundary(&home);
    stop_daemon(&home);
    cleanup.assert_exact_processes_live();
    rt().block_on(async {
        DaemonClient::connect_or_spawn()
            .await
            .expect("restart exact Cargo-built daemon");
    });
    wait_for_reconciled_session_engine(&home, &original.pubkey, 2, log_boundary);
    let after = wait_for_running_generation(&home, &original.pubkey, 2);
    assert_eq!(after.pubkey, original.pubkey);
    assert_eq!(after.agent_slug, agent);
    assert_eq!(
        Store::open(&home.store_path())
            .unwrap()
            .list_session_routes(&original.pubkey)
            .unwrap(),
        [],
        "daemon restart invented a route for an explicitly routeless session"
    );
    cleanup.assert_exact_processes_live();
    assert_memberships_still_absent(&home, &original.pubkey, &root, &child_h, &child_path);
    wait_for_exact_relay_groups(
        &shared_nip29_relay_url(),
        &original.pubkey,
        &BTreeSet::new(),
        Duration::from_secs(25),
    );

    let (statusline, no_route_error) = rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        let statusline = client
            .call(
                "statusline",
                serde_json::json!({ "session": &original.pubkey }),
            )
            .await
            .expect("routeless public statusline");
        let error = client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &original.pubkey,
                    "message": "must not invent a route",
                }),
            )
            .await
            .expect_err("routeless send without channel must fail")
            .to_string();
        (statusline, error)
    });
    assert_eq!(
        statusline["agent"].as_str(),
        Some(public_handle.as_str()),
        "daemon restart changed the public session identity"
    );
    assert_eq!(statusline["channels"], serde_json::json!([]));
    assert!(
        no_route_error.contains("has not joined any channels"),
        "routeless command used an invented route: {no_route_error}"
    );
    let matching_sessions = Store::open(&home.store_path())
        .unwrap()
        .list_sessions()
        .unwrap()
        .into_iter()
        .filter(|row| row.agent_slug == agent)
        .collect::<Vec<_>>();
    assert_eq!(
        matching_sessions.len(),
        1,
        "restart minted a sibling routeless session: {matching_sessions:?}"
    );
    assert_eq!(matching_sessions[0].pubkey, original.pubkey);

    let post_restart_body = format!("still routeless {}", unique_session("post-restart"));
    rt().block_on(publish_user_kind9(
        &root,
        &post_restart_body,
        &original.pubkey,
    ));
    wait_for_injected_log(&log, &post_restart_body);
    assert_memberships_still_absent(&home, &original.pubkey, &root, &child_h, &child_path);
    wait_for_exact_relay_groups(
        &shared_nip29_relay_url(),
        &original.pubkey,
        &BTreeSet::new(),
        Duration::from_secs(25),
    );

    cleanup.finish();
    stop_daemon(&home);
}

fn wait_for_running_generation(home: &Home, pubkey: &str, generation: u64) -> Session {
    let mut found = None;
    assert!(wait_until(Duration::from_secs(25), || {
        found = Store::open(&home.store_path())
            .and_then(|store| store.get_session(pubkey))
            .unwrap_or(None)
            .filter(|session| session.is_running() && session.runtime_generation == generation);
        found.is_some()
    }));
    found.unwrap()
}

fn assert_memberships_still_absent(
    home: &Home,
    pubkey: &str,
    root: &str,
    child_h: &str,
    child_path: &str,
) {
    assert!(wait_until(Duration::from_secs(15), || {
        refresh_channel_members(&format!("#{root}"));
        refresh_channel_members(child_path);
        let store = Store::open(&home.store_path()).unwrap();
        store.list_session_routes(pubkey).unwrap().is_empty()
            && !store.is_channel_member(root, pubkey).unwrap()
            && !store.is_channel_member(child_h, pubkey).unwrap()
    }));
    assert_absent(home, pubkey, root);
    assert_absent(home, pubkey, child_h);
}

fn assert_absent(home: &Home, pubkey: &str, channel: &str) {
    let standing = Store::open(&home.store_path())
        .unwrap()
        .get_session_standing(pubkey, channel)
        .unwrap()
        .expect("standing row");
    assert_eq!(standing.state, mosaico::state::StandingState::Absent);
}
