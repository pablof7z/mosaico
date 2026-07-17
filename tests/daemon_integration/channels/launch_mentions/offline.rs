use super::*;

fn launch_target(home: &Home, agent: &str, channel: &str, work_dir: &Path) -> (String, Session) {
    let pty_id = rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        let response = client
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": channel,
                    "channel": channel,
                    "cwd": work_dir,
                }),
            )
            .await
            .expect("pty_spawn target");
        response["pty_id"].as_str().unwrap().to_string()
    });
    let session = wait_for_alive_session(home, agent, channel);
    wait_for_group_member(home, channel, &session.pubkey);
    let resume = Store::open(&home.store_path())
        .unwrap()
        .native_resume_locator(&session.pubkey, &session.observed_harness)
        .unwrap();
    assert!(resume.is_some(), "launched target must be resumable");
    (pty_id, session)
}

fn start_keeper(home: &Home, channel: &str, work_dir: &Path) {
    rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "keeper",
                        "harness_session": unique_session("keeper"),
                        "cwd": work_dir,
                        "watch_pid": std::process::id(),
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("keeper session_start");
    });
    let keeper = wait_for_alive_session(home, "keeper", channel);
    wait_for_group_member(home, channel, &keeper.pubkey);
}

fn end_target(home: &Home, pubkey: &str) {
    rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        let response = client
            .call("session_kill", serde_json::json!({ "session": pubkey }))
            .await
            .expect("session_kill target");
        assert_eq!(response["killed"], true);
    });
    assert!(wait_until(Duration::from_secs(5), || Store::open(
        &home.store_path()
    )
    .and_then(|store| store.get_session(pubkey))
    .unwrap_or(None)
    .is_some_and(|session| !session.alive)));
}

#[test]
fn operator_kind9_to_offline_session_resumes_the_exact_pubkey() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("kind9-resume");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "offline-kind9";
    let log = home.dir.path().join("offline-injected.log");
    let native_session = unique_session("offline-native");
    let _path = install_opencode_shim(&home, &native_session, &work_dir, &log);
    identity::add_local_agent(home.dir.path(), agent, "offline-test", None, 1)
        .expect("add local agent");

    let (_, original) = launch_target(&home, agent, &channel, &work_dir);
    start_keeper(&home, &channel, &work_dir);
    end_target(&home, &original.pubkey);

    let body = format!("resume exact session {}", unique_session("body"));
    rt().block_on(publish_user_kind9(&channel, &body, &original.pubkey));

    let resumed = wait_for_alive_session(&home, agent, &channel);
    assert_eq!(resumed.pubkey, original.pubkey);
    wait_for_injected_log(&log, &body);

    let store = Store::open(&home.store_path()).unwrap();
    let endpoint = pty_session_for_session(&store, &original.pubkey).expect("resumed endpoint");
    kill_pty(&endpoint);
    stop_daemon(&home);
}

#[test]
fn operator_kind9_to_offline_nonresumable_session_stays_pending_without_sibling_spawn() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("kind9-pending");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "pending-kind9";
    let log = home.dir.path().join("pending-injected.log");
    let native_session = unique_session("pending-native");
    let _path = install_opencode_shim(&home, &native_session, &work_dir, &log);
    identity::add_local_agent(home.dir.path(), agent, "offline-test", None, 1)
        .expect("add local agent");

    let (_, original) = launch_target(&home, agent, &channel, &work_dir);
    start_keeper(&home, &channel, &work_dir);
    end_target(&home, &original.pubkey);
    Store::open(&home.store_path())
        .unwrap()
        .clear_locator_kind(&original.pubkey, "native_resume")
        .unwrap();

    let body = format!("stay with exact session {}", unique_session("body"));
    let event_id = rt().block_on(publish_user_kind9(&channel, &body, &original.pubkey));

    assert!(wait_until(Duration::from_secs(10), || Store::open(
        &home.store_path()
    )
    .and_then(|store| store.peek_pending_for_pubkey(&original.pubkey))
    .is_ok_and(|pending| pending
        .iter()
        .any(|row| row.event_id == event_id))));
    assert!(wait_until(Duration::from_secs(10), || {
        std::fs::read_to_string(home.dir.path().join("daemon.log")).is_ok_and(|text| {
            text.contains("per-session mention target has no native resume locator")
        })
    }));

    let session_count: i64 = rusqlite::Connection::open(home.store_path())
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE agent_slug=?1",
            [agent],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(session_count, 1, "must not mint a sibling session pubkey");
    assert!(
        !Store::open(&home.store_path())
            .unwrap()
            .get_session(&original.pubkey)
            .unwrap()
            .unwrap()
            .alive
    );
    assert!(!std::fs::read_to_string(&log)
        .unwrap_or_default()
        .contains(&body));
    stop_daemon(&home);
}

#[test]
fn operator_kind9_to_stable_agent_starts_the_same_pubkey() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("kind9-stable");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "stable-kind9";
    let log = home.dir.path().join("stable-injected.log");
    let native_session = unique_session("stable-native");
    let _path = install_opencode_shim(&home, &native_session, &work_dir, &log);
    let (identity, _) = identity::save_local_agent(
        home.dir.path(),
        agent,
        identity::LocalAgentUpdate {
            harness: "offline-test".to_string(),
            profile: None,
            per_session_key: Some(false),
            byline: None,
        },
        1,
    )
    .expect("add stable agent");
    let stable_pubkey = identity.pubkey_hex().expect("stable agent pubkey");

    start_keeper(&home, &channel, &work_dir);
    rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        let added = client
            .call(
                "channel_add_member",
                serde_json::json!({
                    "channel": channel,
                    "pubkey": stable_pubkey,
                    "cwd": work_dir,
                }),
            )
            .await
            .expect("channel_add_member stable agent");
        assert_eq!(added["pubkey"], stable_pubkey);
    });
    wait_for_group_member(&home, &channel, &stable_pubkey);

    let body = format!("start stable identity {}", unique_session("body"));
    rt().block_on(publish_user_kind9(&channel, &body, &stable_pubkey));

    let session = wait_for_alive_session(&home, agent, &channel);
    assert_eq!(session.pubkey, stable_pubkey);
    wait_for_injected_log(&log, &body);

    let store = Store::open(&home.store_path()).unwrap();
    let endpoint = pty_session_for_session(&store, &stable_pubkey).expect("stable endpoint");
    kill_pty(&endpoint);
    stop_daemon(&home);
}
