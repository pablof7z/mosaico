use super::*;
use std::path::Path;
use std::time::Duration;

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
            found = Store::open(&home.store_path()).ok().and_then(|s| {
                s.list_running_sessions().ok()?.into_iter().find(|rec| {
                    rec.agent_slug == agent
                        && s.has_session_route(&rec.pubkey, channel).unwrap_or(false)
                })
            });
            found.is_some()
        }),
        "session {agent} in {channel} did not become alive; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<unreadable: {e}>"))
    );
    found.unwrap()
}

#[test]
fn channel_add_session_pulls_live_pty_without_resuming() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let root = unique_session("add-session-root");
    let work_dir = home.dir.path().join(&root);
    add_workspace_mapping(&home, &root, &work_dir);
    let agent = "pulled-live-agent";
    configure_pty_agent(&home, agent, "forever");

    let pty_id = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
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
            .expect("pty_spawn");
        v["pty_id"].as_str().unwrap().to_string()
    });
    let rec = wait_for_alive(&home, agent, &root);
    let spawned_meta = mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|metadata| metadata.id == pty_id)
        .expect("spawned PTY metadata");
    assert_eq!(spawned_meta.command, ["opencode", "forever"]);

    let side_path = format!("/{root}/side");
    let created = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channel_create",
            serde_json::json!({
                "channel": format!("/{root}/side"),
                "about": "side channel",
                "cwd": &work_dir,
            }),
        )
        .await
        .expect("channel_create")
    });
    assert_eq!(created["channel"], side_path);
    assert_eq!(created["joined"].as_bool(), Some(false));
    let side = Store::open(&home.store_path())
        .unwrap()
        .channel_id_for_name(&root, "side")
        .unwrap()
        .expect("side channel id");
    let pty_count = mosaico::pty::read_all_metadata().len();
    let before_invite = Store::open(&home.store_path())
        .unwrap()
        .get_session(&rec.pubkey)
        .unwrap()
        .expect("live session before invite");
    assert!(
        before_invite.is_running(),
        "target stopped before invite: {before_invite:?}; pty_live={}; daemon_log={}",
        mosaico::pty::is_live(&pty_id),
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );

    let added = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "invite",
            serde_json::json!({
                "channel": &side_path,
                "session": &rec.pubkey,
                "cwd": &work_dir,
            }),
        )
        .await
        .expect("invite live session")
    });
    assert_eq!(added["pty_id"], pty_id);
    assert_eq!(added["channel"], side_path);
    assert_eq!(mosaico::pty::read_all_metadata().len(), pty_count);

    assert!(
        wait_until(Duration::from_secs(25), || {
            refresh_channel_members(&side_path);
            Store::open(&home.store_path())
                .map(|s| {
                    s.has_session_route(&rec.pubkey, &side).unwrap_or(false)
                        && s.is_channel_member(&side, &rec.pubkey).unwrap_or(false)
                })
                .unwrap_or(false)
        }),
        "live session was not joined/member in {side}; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<unreadable: {e}>"))
    );

    let store = Store::open(&home.store_path()).unwrap();
    assert!(store.has_session_route(&rec.pubkey, &root).unwrap());

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}

#[test]
fn direct_fallback_reattaches_a_live_handle_and_rejects_launch_only_options() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let root = unique_session("launch-existing-root");
    let work_dir = home.dir.path().join(&root);
    add_workspace_mapping(&home, &root, &work_dir);
    let agent = "launch-existing-agent";
    configure_pty_agent(&home, agent, "forever");
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
            .expect("pty_spawn");
        (
            spawned["pty_id"].as_str().expect("pty id").to_string(),
            spawned["handle"]
                .as_str()
                .expect("public session handle")
                .to_string(),
        )
    });
    let _ = wait_for_alive(&home, agent, &root);

    let attached = run_cli_with_env_in_dir(&home, &[&handle], &[], &work_dir);
    assert!(attached.status.success(), "{attached:?}");
    assert!(
        String::from_utf8_lossy(&attached.stderr).contains(&format!("Attached to {handle}")),
        "{}",
        String::from_utf8_lossy(&attached.stderr)
    );
    assert!(mosaico::pty::is_live(&pty_id));

    let renamed = run_cli_with_env_in_dir(&home, &[&handle, "--name", "hi"], &[], &work_dir);
    assert!(!renamed.status.success());
    assert!(
        String::from_utf8_lossy(&renamed.stderr).contains("no available agent"),
        "{}",
        String::from_utf8_lossy(&renamed.stderr)
    );

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}
