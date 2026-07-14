use super::*;

#[test]
fn operator_kind9_injects_into_working_launch_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("kind9-launch");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let log = home.dir.path().join("launch-injected.log");
    let native_session = unique_session("launch-native");
    let agent = "launch-kind9";

    let pty_id = rt().block_on(async {
        let mut c = DaemonClient::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": channel,
                    "channel": channel,
                    "cwd": work_dir,
                    "launch": {"kind": "pty-command", "argv": harness_command(&native_session, &work_dir, &log)},
                }),
            )
            .await
            .expect("pty_spawn");
        v["pty_id"].as_str().unwrap().to_string()
    });

    let rec = wait_for_alive_session(&home, agent, &channel);
    wait_for_group_member(&home, &channel, &rec.pubkey);
    Store::open(&home.store_path())
        .unwrap()
        .set_working(&rec.pubkey, true, tenex_edge::util::now_secs())
        .expect("mark launch session working");

    let body = format!("operator relay injection {}", unique_session("body"));
    rt().block_on(async {
        publish_user_kind9(&channel, &body, &rec.pubkey).await;
    });
    wait_for_injected_log(&log, &body);

    let store = Store::open(&home.store_path()).unwrap();
    let messages = chat_in_channel(&store, &channel);
    assert!(
        messages
            .iter()
            .any(|m| m.content == body && m.pubkey == pubkey_of(EXAMPLE_USER_NSEC)),
        "operator kind:9 should be materialized as user-authored chat"
    );

    kill_pty(&pty_id);
    stop_daemon(&home);
}
