use super::*;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

fn add_workspace_mapping(home: &Home, channel: &str, path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    let mut workspaces = serde_json::Map::new();
    workspaces.insert(
        channel.to_string(),
        serde_json::Value::String(path.to_string_lossy().into_owned()),
    );
    std::fs::write(
        home.dir.path().join("workspaces.json"),
        serde_json::Value::Object(workspaces).to_string(),
    )
    .unwrap();
}

fn local_routes(home: &Home, pubkey: &str) -> BTreeSet<String> {
    session_routes(&Store::open(&home.store_path()).unwrap(), pubkey)
        .into_iter()
        .collect()
}

fn public_channels(value: &serde_json::Value) -> BTreeSet<String> {
    value["channels"]
        .as_array()
        .expect("statusline channels")
        .iter()
        .map(|channel| channel.as_str().expect("public channel path").to_string())
        .collect()
}

fn exact_state_is_current(
    home: &Home,
    pubkey: &str,
    generation: u64,
    routes: &BTreeSet<String>,
) -> bool {
    let Ok(store) = Store::open(&home.store_path()) else {
        return false;
    };
    store
        .get_session(pubkey)
        .unwrap_or(None)
        .is_some_and(|session| session.is_running() && session.runtime_generation == generation)
        && local_routes(home, pubkey) == *routes
        && routes.iter().all(|route| {
            store
                .get_session_standing(pubkey, route)
                .unwrap()
                .is_some_and(|standing| standing.state == mosaico::state::StandingState::Member)
        })
}

#[test]
fn daemon_restart_preserves_exact_multiroute_identity_and_relay_standing() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);
    let relay = shared_nip29_relay_url();
    let root = unique_session("restart-routes");
    let work_dir = home.dir.path().join(&root);
    add_workspace_mapping(&home, &root, &work_dir);
    let agent = "restart-routes-agent";
    configure_pty_agent(&home, agent, "capture-input");

    let (pty_id, handle) = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let spawned = client
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": &root,
                    "channel": &root,
                    "cwd": &work_dir,
                }),
            )
            .await
            .expect("spawn hosted session");
        (
            spawned["pty_id"].as_str().expect("pty id").to_string(),
            spawned["handle"]
                .as_str()
                .expect("public session handle")
                .to_string(),
        )
    });
    let cleanup = PtyProcessGuard::capture(&pty_id);
    let session = {
        let mut found = None;
        assert!(wait_until(Duration::from_secs(25), || {
            found = Store::open(&home.store_path())
                .and_then(|store| store.list_running_sessions())
                .unwrap_or_default()
                .into_iter()
                .find(|session| session.agent_slug == agent);
            found.is_some()
        }));
        found.unwrap()
    };

    let child_names = [unique_session("implementation"), unique_session("review")];
    let child_paths = child_names
        .iter()
        .map(|name| format!("/{root}/{name}"))
        .collect::<Vec<_>>();
    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        for path in &child_paths {
            let created = client
                .call(
                    "channel_create",
                    serde_json::json!({
                        "channel": path,
                        "agents": [],
                        "session": &session.pubkey,
                    }),
                )
                .await
                .unwrap_or_else(|error| panic!("create {path}: {error:#}"));
            assert_eq!(created["joined"].as_bool(), Some(true));
        }
    });

    let child_ids = {
        let store = Store::open(&home.store_path()).unwrap();
        child_names
            .iter()
            .map(|name| {
                store
                    .channel_id_for_name(&root, name)
                    .unwrap()
                    .unwrap_or_else(|| panic!("missing opaque channel id for {name}"))
            })
            .collect::<Vec<_>>()
    };
    let expected_routes = std::iter::once(root.clone())
        .chain(child_ids)
        .collect::<BTreeSet<_>>();
    let expected_paths = std::iter::once(format!("/{root}"))
        .chain(child_paths)
        .collect::<BTreeSet<_>>();
    assert!(
        wait_until(Duration::from_secs(25), || {
            for path in &expected_paths {
                refresh_channel_members(path);
            }
            exact_state_is_current(
                &home,
                &session.pubkey,
                session.runtime_generation,
                &expected_routes,
            )
        }),
        "three-route standing did not converge; routes={:?}; daemon_log={}",
        local_routes(&home, &session.pubkey),
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    wait_for_exact_relay_groups(
        &relay,
        &session.pubkey,
        &expected_routes,
        Duration::from_secs(25),
    );

    let before = rt().block_on(async {
        Client::connect_or_spawn()
            .await
            .expect("connect")
            .call(
                "statusline",
                serde_json::json!({ "session": &session.pubkey }),
            )
            .await
            .expect("public statusline before restart")
    });
    assert_eq!(before["agent"].as_str(), Some(handle.as_str()));
    assert_eq!(public_channels(&before), expected_paths);

    let log_boundary = daemon_log_boundary(&home);
    stop_daemon(&home);
    cleanup.assert_exact_processes_live();
    let after = rt().block_on(async {
        let mut client = Client::connect_or_spawn()
            .await
            .expect("restart exact Cargo-built daemon");
        wait_for_reconciled_session_engine(
            &home,
            &session.pubkey,
            session.runtime_generation,
            log_boundary,
        );
        client
            .call(
                "statusline",
                serde_json::json!({ "session": &session.pubkey }),
            )
            .await
            .expect("public statusline after restart")
    });
    assert!(
        exact_state_is_current(
            &home,
            &session.pubkey,
            session.runtime_generation,
            &expected_routes,
        ),
        "the reconciled session lost identity, generation, routes, or standing"
    );
    assert_eq!(after["agent"].as_str(), Some(handle.as_str()));
    assert_eq!(public_channels(&after), expected_paths);
    cleanup.assert_exact_processes_live();

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
        "restart minted a sibling session: {matching_sessions:?}"
    );
    assert_eq!(matching_sessions[0].pubkey, session.pubkey);
    wait_for_exact_relay_groups(
        &relay,
        &session.pubkey,
        &expected_routes,
        Duration::from_secs(25),
    );

    let deliveries = expected_routes
        .iter()
        .enumerate()
        .map(|(index, route)| {
            (
                route.clone(),
                format!("post-restart route {index} {}", unique_session("delivery")),
            )
        })
        .collect::<Vec<_>>();
    rt().block_on(async {
        for (route, body) in &deliveries {
            publish_addressed_chat(&relay, EXAMPLE_USER_NSEC, route, body, &session.pubkey).await;
        }
    });
    let capture = home.dir.path().join("captured-pty-input");
    assert!(
        wait_until(Duration::from_secs(25), || {
            let input = std::fs::read_to_string(&capture).unwrap_or_default();
            deliveries.iter().all(|(_, body)| input.contains(body))
        }),
        "the re-adopted exact PTY did not receive work through every retained route; \
         capture={}; daemon_log={}",
        std::fs::read_to_string(&capture).unwrap_or_default(),
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );

    let ambiguity = rt().block_on(async {
        Client::connect_or_spawn()
            .await
            .expect("connect")
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &session.pubkey,
                    "message": "must not select an implicit route",
                }),
            )
            .await
            .expect_err("multi-route send without a channel must remain ambiguous")
            .to_string()
    });
    assert!(
        ambiguity.contains("channel send is ambiguous"),
        "a route was implicitly selected after restart: {ambiguity}"
    );
    assert!(exact_state_is_current(
        &home,
        &session.pubkey,
        session.runtime_generation,
        &expected_routes,
    ));
    wait_for_exact_relay_groups(
        &relay,
        &session.pubkey,
        &expected_routes,
        Duration::from_secs(25),
    );

    cleanup.finish();
    stop_daemon(&home);
}
