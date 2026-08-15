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
    let original_routes = store.list_session_routes(&session.pubkey).unwrap();
    assert_eq!(identity.slug, "codex");
    assert_eq!(identity.handle, "forensic-researcher-codex");
    let latest_live_presence_receipt = |after: i64| {
        Store::open(&home.store_path())
            .ok()?
            .latest_receipts_for_surface("status", 100)
            .ok()?
            .into_iter()
            .find(|receipt| {
                receipt.id > after
                    && (receipt.commands.contains("\"opened\"")
                        || receipt.commands.contains("\"admitted\""))
                    && serde_json::from_str::<serde_json::Value>(&receipt.changed_summary)
                        .is_ok_and(|summary| summary["pubkey"] == session.pubkey)
            })
            .map(|receipt| receipt.id)
    };
    let mut first_open_receipt = None;
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            first_open_receipt = latest_live_presence_receipt(0);
            first_open_receipt.is_some()
        }),
        "fresh session published no live presence receipt"
    );

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
    let receipt_cursor = Store::open(&home.store_path())
        .unwrap()
        .latest_receipts_for_surface("status", 1)
        .unwrap()
        .into_iter()
        .next()
        .map(|receipt| receipt.id)
        .unwrap_or(first_open_receipt.unwrap());

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
    let resumed_session = wait_for_alive(&home, "codex", &channel);
    assert_eq!(resumed_session.pubkey, session.pubkey);
    assert_eq!(
        resumed_session.runtime_generation,
        session.runtime_generation + 1
    );
    assert_eq!(resumed_session.agent_slug, session.agent_slug);
    assert_eq!(resumed_session.work_root, session.work_root);
    assert_eq!(resumed_session.observed_harness, session.observed_harness);
    assert_eq!(resumed_session.admitted_preset, session.admitted_preset);
    assert_eq!(
        resumed_session.admitted_transport,
        session.admitted_transport
    );
    let resumed_store = Store::open(&home.store_path()).unwrap();
    assert_eq!(
        resumed_store
            .list_session_routes(&resumed_session.pubkey)
            .unwrap(),
        original_routes
    );
    assert_eq!(
        resumed_store
            .list_running_sessions()
            .unwrap()
            .iter()
            .filter(|record| record.pubkey == resumed_session.pubkey)
            .count(),
        1
    );
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            latest_live_presence_receipt(receipt_cursor).is_some()
        }),
        "resumed session published no live presence receipt"
    );
    let _ = mosaico::pty::kill(&resumed_meta.id);
    stop_daemon(&home);
}
