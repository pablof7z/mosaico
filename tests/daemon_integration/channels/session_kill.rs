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

fn wait_for_alive(home: &Home, agent: &str, channel: &str) -> tenex_edge::state::Session {
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
        "session {agent} in {channel} did not become alive"
    );
    found.unwrap()
}

#[test]
fn session_kill_stops_pty_session_and_marks_offline() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("kill-pty");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "kill-agent";

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
                    "launch": {"kind": "pty-command", "argv": no_hook_command()},
                }),
            )
            .await
            .expect("pty_spawn");
        v["pty_id"].as_str().unwrap().to_string()
    });
    let rec = wait_for_alive(&home, agent, &channel);
    let selector = tenex_edge::idref::npub(&rec.pubkey).expect("session npub");

    let killed = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call("session_kill", serde_json::json!({ "session": &selector }))
            .await
            .expect("session_kill")
    });
    assert_eq!(killed["killed"].as_bool(), Some(true));
    assert_eq!(killed["ended"].as_bool(), Some(true));

    assert!(
        wait_until(Duration::from_secs(5), || {
            !tenex_edge::pty::is_live(&pty_id)
                && !tenex_edge::pty::read_all_metadata()
                    .iter()
                    .any(|meta| meta.id == pty_id)
        }),
        "pty supervisor should stop and remove metadata"
    );
    assert!(
        wait_until(Duration::from_secs(5), || {
            let store = Store::open(&home.store_path()).unwrap();
            let offline = store
                .get_session(&rec.pubkey)
                .unwrap()
                .map(|row| !row.alive)
                .unwrap_or(false);
            offline && pty_session_for_session(&store, &rec.pubkey).is_none()
        }),
        "session should be offline and detached from pty alias"
    );

    stop_daemon(&home);
}
