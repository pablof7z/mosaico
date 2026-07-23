use super::*;

#[test]
fn stale_daemon_cwd_is_actionable_and_does_not_break_explicit_launch_cwd() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let daemon_cwd = home.dir.path().join("deleted-daemon-cwd");
    std::fs::create_dir(&daemon_cwd).unwrap();
    let started = run_cli_with_env_in_dir(&home, &["agents", "list"], &[], &daemon_cwd);
    assert!(
        started.status.success(),
        "daemon startup failed: {}",
        String::from_utf8_lossy(&started.stderr)
    );
    std::fs::remove_dir(&daemon_cwd).unwrap();

    let missing_cwd_error = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.unwrap();
        client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({"agent": "missing-cwd-probe"}),
                    "opencode",
                ),
            )
            .await
            .unwrap_err()
            .to_string()
    });
    assert!(
        missing_cwd_error.contains(
            "the mosaico daemon can no longer access its working directory; \
             restart it with `mosaico daemon restart` and try again"
        ),
        "{missing_cwd_error}"
    );

    let channel = unique_session("stale-daemon-cwd");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "stale-daemon-cwd-agent";
    configure_pty_agent(&home, agent, "forever");
    let launched = run_cli_with_env_in_dir(&home, &[agent], &[], &work_dir);
    assert!(
        launched.status.success(),
        "explicit launch cwd should bypass stale daemon cwd: {}",
        String::from_utf8_lossy(&launched.stderr)
    );

    let session = wait_for_alive(&home, agent, &channel);
    let pty_id = Store::open(&home.store_path())
        .unwrap()
        .locators_for_pubkey(&session.pubkey)
        .unwrap()
        .into_iter()
        .find(|locator| locator.locator_kind == "pty")
        .map(|locator| locator.locator_value)
        .expect("launched session PTY locator");
    mosaico::pty::kill(&pty_id).unwrap();
    stop_daemon(&home);
}
