use super::*;
use std::path::Path;
use std::time::Duration;

#[path = "pty_bootstrap/launch.rs"]
mod launch;
#[path = "pty_bootstrap/named.rs"]
mod named;

fn add_workspace_mapping(home: &Home, channel: &str, path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    let map_path = home.dir.path().join("workspaces.json");
    let mut map = std::fs::read_to_string(&map_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&s).ok())
        .unwrap_or_default();
    map.insert(
        channel.to_string(),
        serde_json::Value::String(path.to_string_lossy().to_string()),
    );
    std::fs::write(&map_path, serde_json::to_string(&map).unwrap()).unwrap();
}

fn wait_for_alive(home: &Home, agent: &str, channel: &str) -> mosaico::state::Session {
    let mut found = None;
    assert!(
        wait_until(Duration::from_secs(25), || {
            found = Store::open(&home.store_path())
                .and_then(|s| s.list_alive_sessions())
                .unwrap_or_default()
                .into_iter()
                .find(|rec| rec.agent_slug == agent && rec.channel_h == channel);
            found.is_some()
        }),
        "session {agent} in {channel} did not become alive; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<unreadable: {e}>"))
    );
    found.unwrap()
}

fn pty_meta(pty_id: &str) -> mosaico::pty::LaunchMetadata {
    mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|meta| meta.id == pty_id)
        .expect("pty metadata")
}

#[test]
fn pty_spawn_bootstraps_session_without_child_session_start_hook() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("pty-bootstrap");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "no-hook-agent";
    configure_pty_agent(&home, agent, "forever");

    let pty_id = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": &channel,
                    "channel": &channel,
                    "cwd": &work_dir,
                }),
            )
            .await
            .expect("pty_spawn");
        v["pty_id"].as_str().unwrap().to_string()
    });

    let rec = wait_for_alive(&home, agent, &channel);
    let store = Store::open(&home.store_path()).unwrap();
    let locators = store.locators_for_pubkey(&rec.pubkey).unwrap();
    assert_eq!(
        locators
            .iter()
            .find(|locator| locator.locator_kind == "pty")
            .map(|locator| locator.locator_value.clone()),
        Some(pty_id.clone())
    );
    // Membership is relay-materialized (the daemon publishes the 39002 snapshot
    // and materializes it back from the relay), so poll for it rather than
    // asserting on a single refresh — otherwise this races the propagation.
    assert!(
        wait_until(Duration::from_secs(25), || {
            refresh_channel_members(&channel);
            Store::open(&home.store_path())
                .map(|s| s.is_channel_member(&channel, &rec.pubkey).unwrap_or(false))
                .unwrap_or(false)
        }),
        "agent {} did not materialize as a member of {channel}; daemon_log={}",
        rec.pubkey,
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<unreadable: {e}>"))
    );

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}

#[test]
fn late_session_start_hook_reasserts_pty_bootstrap_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("pty-reassert");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "late-hook-agent";
    configure_pty_agent(&home, agent, "forever");

    let pty_id = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": &channel,
                    "channel": &channel,
                    "cwd": &work_dir,
                }),
            )
            .await
            .expect("pty_spawn");
        v["pty_id"].as_str().unwrap().to_string()
    });
    let first = wait_for_alive(&home, agent, &channel);
    let meta = pty_meta(&pty_id);
    let native_session = unique_session("native-hook");

    let reasserted = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({
                "agent": agent,
                "harness": "codex",
                "harness_session": &native_session,
                "cwd": &work_dir,
                "channel": &channel,
                "watch_pid": i32::try_from(meta.supervisor_pid).ok(),
                "pty_session": &pty_id,
            }),
        )
        .await
        .expect("session_start")
    });

    assert_eq!(reasserted["pubkey"].as_str(), Some(first.pubkey.as_str()));
    let store = Store::open(&home.store_path()).unwrap();
    let alive = store
        .list_alive_sessions()
        .unwrap()
        .into_iter()
        .filter(|rec| rec.agent_slug == agent && rec.channel_h == channel)
        .collect::<Vec<_>>();
    assert_eq!(
        alive.len(),
        1,
        "late hook should not mint a duplicate session"
    );
    assert_eq!(
        store
            .resolve_pubkey_by_locator("codex", "native_resume", &native_session,)
            .unwrap()
            .as_deref(),
        Some(first.pubkey.as_str())
    );

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}

#[test]
fn codex_hook_reasserts_launch_session_from_pty_anchor_without_native_id() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("pty-codex-hook");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "codex";
    configure_pty_agent(&home, agent, "forever");

    let pty_id = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": &channel,
                    "channel": &channel,
                    "cwd": &work_dir,
                }),
            )
            .await
            .expect("pty_spawn");
        v["pty_id"].as_str().unwrap().to_string()
    });
    let first = wait_for_alive(&home, agent, &channel);
    let meta = pty_meta(&pty_id);

    let out = run_cli_stdin_with_env_in_dir(
        &home,
        &["harness", "hook", "codex", "--type", "session-start"],
        "",
        &[
            ("MOSAICO_AGENT", agent),
            ("MOSAICO_PTY_SESSION", pty_id.as_str()),
            ("MOSAICO_PTY_SOCKET", meta.socket.as_str()),
            ("MOSAICO_INIT_PROGRESS", "0"),
        ],
        &work_dir,
    );
    assert!(
        out.status.success(),
        "codex session-start hook failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let store = Store::open(&home.store_path()).unwrap();
    assert_eq!(
        store
            .resolve_pubkey_by_locator("codex", "pty", &pty_id)
            .unwrap()
            .as_deref(),
        Some(first.pubkey.as_str())
    );
    let alive = store
        .list_alive_sessions()
        .unwrap()
        .into_iter()
        .filter(|rec| rec.agent_slug == agent && rec.channel_h == channel)
        .collect::<Vec<_>>();
    assert_eq!(alive.len(), 1, "codex hook should not mint a duplicate");

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}
