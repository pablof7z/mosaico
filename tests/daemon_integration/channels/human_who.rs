use super::*;

#[test]
fn who_without_agent_anchor_returns_human_fabric_view_with_other_roots() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let channel = unique_session("human-who");
    let other_root = unique_session("human-other");
    let workspaces = tempfile::Builder::new()
        .prefix("mosaico-human-who-")
        .tempdir_in("/var/tmp")
        .unwrap();
    let current_path = workspaces.path().join("current");
    let other_path = workspaces.path().join("other");
    std::fs::create_dir_all(&current_path).unwrap();
    std::fs::create_dir_all(&other_path).unwrap();
    initialize_workspace_root(&channel, current_path.to_str().unwrap());
    initialize_workspace_root(&other_root, other_path.to_str().unwrap());
    wait_for_channel_metadata(&home, &channel);
    wait_for_channel_metadata(&home, &other_root);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "reviewer",
                    "harness_session": unique_session("human-reviewer"),
                    "cwd": other_path,
                    "channel": &other_root,
                    "watch_pid": std::process::id(),
                }),
                "claude-code",
            ),
        )
        .await
        .expect("start reviewer in the other workspace");
        let v = c
            .call(
                "who",
                serde_json::json!({
                    "workspace": &channel,
                    "human_color": false
                }),
            )
            .await
            .expect("human who should render");

        let human = v["fabric_human"]
            .as_str()
            .expect("human who should include fabric_human");
        assert!(human.contains(&format!("#{channel}")), "got: {human}");
        assert!(human.contains("Other workspaces"), "got: {human}");
        assert!(human.contains(&other_root), "got: {human}");
        assert!(human.contains("reviewer"), "got: {human}");
        assert!(human.contains("1 agent"), "got: {human}");
        assert!(!human.contains("<mosaico>"), "got: {human}");
        assert!(v.get("fabric").is_none(), "who must not expose agent XML");
    });

    stop_daemon(&home);
}
