use super::*;

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
            ("MOSAICO_OBSERVED_HARNESS", "opencode"),
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
            .resolve_pubkey_by_locator("opencode", "pty", &pty_id)
            .unwrap()
            .as_deref(),
        Some(first.pubkey.as_str())
    );
    let alive = store
        .list_running_sessions()
        .unwrap()
        .into_iter()
        .filter(|rec| {
            rec.agent_slug == agent
                && store
                    .has_session_route(&rec.pubkey, &channel)
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        alive.len(),
        1,
        "codex hook should not mint a duplicate: {:?}",
        alive
            .iter()
            .map(|rec| (
                rec.pubkey.as_str(),
                rec.observed_harness.as_str(),
                rec.claimed_harness.as_str(),
                rec.endpoint_provenance.as_str(),
            ))
            .collect::<Vec<_>>()
    );
    let reasserted = &alive[0];
    assert_eq!(reasserted.observed_harness, "opencode");
    assert_eq!(reasserted.claimed_harness, "codex");
    assert_eq!(reasserted.admitted_preset, "test-pty");
    assert_eq!(reasserted.admitted_transport, "pty");
    assert_eq!(reasserted.endpoint_provenance, "launch");

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}
