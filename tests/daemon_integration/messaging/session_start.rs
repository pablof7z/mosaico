use super::*;

#[test]
fn session_start_runs_engine_and_records_alive_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let pubkey = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "session_start",
                hook_session_start(serde_json::json!({"agent": "coder", "harness_session": "sess-int-1", "cwd": "/tmp"}), "claude-code"),
            )
            .await
            .expect("session_start");
        v["pubkey"].as_str().unwrap().to_string()
    });
    // The public identity owns the row; the harness id is only a typed locator.
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session(&pubkey)
        .unwrap()
        .expect("session row by pubkey");
    assert_eq!(rec.pubkey, pubkey);
    assert_eq!(
        store
            .resolve_pubkey_by_locator("claude-code", "native_resume", "sess-int-1",)
            .unwrap()
            .as_deref(),
        Some(pubkey.as_str())
    );
    assert!(rec.is_running());
    assert_eq!(rec.agent_slug, "coder");
    assert!(
        wait_until(Duration::from_secs(25), || Store::open(&home.store_path())
            .and_then(|store| store.has_session_route(&pubkey, "tmp"))
            .unwrap_or(false)),
        "session did not finish joining /tmp"
    );
    assert!(
        wait_until(Duration::from_secs(25), || super::channel_has_members(
            "#tmp",
            &[&pubkey]
        )),
        "session did not reach the NMP-delivered #tmp roster"
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.unwrap();
        let v = c
            .call(
                "who",
                serde_json::json!({"all": true, "all_workspaces": true}),
            )
            .await
            .unwrap();
        let rows = v["rows"].as_array().unwrap();
        assert!(
            rows.iter()
                .any(|r| r["pubkey"] == pubkey.as_str() && r["source"] == "Local"),
            "who rows: {rows:?}"
        );
    });

    stop_daemon(&home);
}

#[test]
fn session_start_replaces_prior_session_for_same_host_pid() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    let pid = std::process::id() as i32;

    let (old_pubkey, new_pubkey) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v1 = c
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "claude",
                        "harness_session": "old-session",
                        "cwd": "/tmp",
                        "watch_pid": pid
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("first session_start");
        let v2 = c
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "claude",
                        "harness_session": "new-session",
                        "cwd": "/tmp",
                        "watch_pid": pid
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("second session_start");
        (
            v1["pubkey"].as_str().unwrap().to_string(),
            v2["pubkey"].as_str().unwrap().to_string(),
        )
    });

    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        !store
            .get_session(&old_pubkey)
            .unwrap()
            .unwrap()
            .is_running(),
        "old session should be marked dead"
    );
    assert!(
        store
            .get_session(&new_pubkey)
            .unwrap()
            .unwrap()
            .is_running(),
        "new session should remain alive"
    );
    assert!(
        wait_until(Duration::from_secs(25), || Store::open(&home.store_path())
            .and_then(|store| store.has_session_route(&new_pubkey, "tmp"))
            .unwrap_or(false)),
        "replacement session did not finish joining /tmp"
    );
    assert!(
        wait_until(Duration::from_secs(25), || super::channel_has_members(
            "#tmp",
            &[&new_pubkey]
        )),
        "replacement session did not reach the NMP-delivered #tmp roster"
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.unwrap();
        let v = c
            .call(
                "who",
                serde_json::json!({"all": true, "all_workspaces": true}),
            )
            .await
            .unwrap();
        let rows = v["rows"].as_array().unwrap();
        let old = rows
            .iter()
            .find(|row| row["pubkey"] == old_pubkey.as_str())
            .expect("--all should retain the stopped predecessor as history");
        assert!(
            old["dormant"] == true && old["state"] == "offline",
            "stopped predecessor must be explicitly dormant/offline: {old:?}"
        );
        assert!(
            rows.iter().any(|r| r["pubkey"] == new_pubkey.as_str()),
            "new session missing from who rows: {rows:?}"
        );
    });

    stop_daemon(&home);
}
