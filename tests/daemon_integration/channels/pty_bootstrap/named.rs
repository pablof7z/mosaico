use super::*;

#[test]
fn pty_spawn_uses_requested_public_name_and_rejects_conflict() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("named-launch");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let session_name = "forensic-researcher";
    configure_pty_agent(&home, "codex", "forever");

    let pty_id = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": "codex",
                    "root": &channel,
                    "channel": &channel,
                    "cwd": &work_dir,
                    "session_name": session_name,
                }),
            )
            .await
            .expect("named pty_spawn");
        v["pty_id"].as_str().expect("pty_id").to_string()
    });
    let session = wait_for_alive(&home, "codex", &channel);
    let store = Store::open(&home.store_path()).unwrap();
    let identity = store
        .session_identity(&session.pubkey)
        .unwrap()
        .expect("named session identity");
    assert_eq!(identity.slug, "codex");
    assert_eq!(identity.handle, "forensic-researcher-codex");

    let error = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "pty_spawn",
            serde_json::json!({
                "agent": "codex",
                "root": &channel,
                "channel": &channel,
                "cwd": &work_dir,
                "session_name": session_name,
            }),
        )
        .await
        .expect_err("a duplicate public name must be rejected")
    });
    assert!(
        format!("{error:#}").contains("forensic-researcher-codex"),
        "unexpected error: {error:#}"
    );

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}
