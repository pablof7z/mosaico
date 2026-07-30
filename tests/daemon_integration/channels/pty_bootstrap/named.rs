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

    let native_id = unique_session("named-native");
    Store::open(&home.store_path())
        .unwrap()
        .set_native_resume_locator(
            &session.pubkey,
            &session.observed_harness,
            &native_id,
            mosaico::util::now_secs(),
        )
        .unwrap();
    mosaico::pty::kill(&pty_id).unwrap();
    assert!(
        wait_until(std::time::Duration::from_secs(10), || {
            Store::open(&home.store_path())
                .ok()
                .and_then(|store| store.get_session(&session.pubkey).ok().flatten())
                .is_some_and(|record| !record.is_running())
        }),
        "named session did not stop before resume"
    );

    let resumed =
        run_cli_with_env_in_dir(&home, &[&identity.handle, "--", "--yolo"], &[], &work_dir);
    assert!(
        resumed.status.success(),
        "named resume failed: {}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&resumed.stderr).contains("Resumed forensic-researcher-codex"),
        "unexpected resume output: {}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed_meta = mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|meta| meta.agent == "codex" && meta.root == channel)
        .expect("resumed named PTY metadata");
    assert_eq!(
        resumed_meta.command,
        ["opencode", "forever", "--session", &native_id, "--yolo"]
    );
    let _ = mosaico::pty::kill(&resumed_meta.id);
    stop_daemon(&home);
}
