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

fn no_hook_command() -> Vec<String> {
    vec![
        "sh".to_string(),
        "-lc".to_string(),
        "while true; do sleep 1; done".to_string(),
    ]
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

#[test]
fn channel_add_session_pulls_live_pty_without_resuming() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let root = unique_session("add-session-root");
    let work_dir = home.dir.path().join(&root);
    add_workspace_mapping(&home, &root, &work_dir);
    let agent = "pulled-live-agent";

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
                    "launch": {"kind": "pty-command", "argv": no_hook_command()},
                }),
            )
            .await
            .expect("pty_spawn");
        v["pty_id"].as_str().unwrap().to_string()
    });
    let rec = wait_for_alive(&home, agent, &root);

    let side = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "channel_create",
                serde_json::json!({
                    "parent": &root,
                    "name": "side",
                    "about": "side channel",
                    "cwd": &work_dir,
                }),
            )
            .await
            .expect("channel_create");
        v["child_h"].as_str().unwrap().to_string()
    });
    let pty_count = mosaico::pty::read_all_metadata().len();

    let added = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "invite",
            serde_json::json!({
                "channel": format!("@{side}"),
                "session": &rec.pubkey,
                "cwd": &work_dir,
            }),
        )
        .await
        .expect("invite live session")
    });
    assert_eq!(added["pty_id"], pty_id);
    assert_eq!(mosaico::pty::read_all_metadata().len(), pty_count);

    assert!(
        wait_until(Duration::from_secs(25), || {
            refresh_channel_members(&side);
            Store::open(&home.store_path())
                .map(|s| {
                    s.is_session_joined_channel(&rec.pubkey, &side)
                        .unwrap_or(false)
                        && s.is_channel_member(&side, &rec.pubkey).unwrap_or(false)
                })
                .unwrap_or(false)
        }),
        "live session was not joined/member in {side}; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<unreadable: {e}>"))
    );

    let active = Store::open(&home.store_path())
        .unwrap()
        .get_session(&rec.pubkey)
        .unwrap()
        .expect("session row after invite")
        .channel_h;
    assert_eq!(active, root);

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}

#[test]
fn launch_existing_reattaches_a_live_handle() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let root = unique_session("launch-existing-root");
    let work_dir = home.dir.path().join(&root);
    add_workspace_mapping(&home, &root, &work_dir);
    let agent = "launch-existing-agent";
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
                    "launch": {"kind": "pty-command", "argv": no_hook_command()},
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

    let launched = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "pty_launch_existing",
                serde_json::json!({ "session": handle }),
            )
            .await
            .expect("reattach live session")
    });
    assert_eq!(launched["action"], "attached");
    assert_eq!(launched["pty_id"], pty_id);
    assert_eq!(launched["handle"], handle);

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}
