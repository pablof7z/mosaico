use super::*;
use std::time::Duration;

#[test]
fn session_start_without_mosaico_private_key_generates_key_and_provisions_channel() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec_without_backend_key(&home, false);
    let sid = unique_session("sess-autokey");

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({"agent": "coder", "harness_session": &sid, "cwd": "/tmp"}),
                "claude-code",
            ),
        )
        .await
        .expect("session_start should generate a management key and provision");
    });

    assert!(
        wait_until(Duration::from_secs(25), || {
            std::fs::read_to_string(home.dir.path().join("config.json"))
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|cfg| cfg["mosaicoPrivateKey"].as_str().map(str::to_string))
                .is_some()
        }),
        "background readiness should generate mosaicoPrivateKey"
    );
    let cfg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.dir.path().join("config.json")).unwrap(),
    )
    .unwrap();
    let generated = cfg["mosaicoPrivateKey"]
        .as_str()
        .expect("generated mosaicoPrivateKey");
    let backend_pk = pubkey_of(generated);
    let user_pk = pubkey_of(EXAMPLE_USER_NSEC);

    assert!(
        wait_until(Duration::from_secs(25), || {
            refresh_channel_members("#tmp");
            let members = Store::open(&home.store_path())
                .and_then(|store| store.list_channel_members("tmp"))
                .unwrap_or_default();
            members
                .iter()
                .any(|m| m.pubkey == backend_pk && m.role == "admin")
                && members
                    .iter()
                    .any(|m| m.pubkey == user_pk && m.role == "admin")
        }),
        "background readiness should grant generated management and user admin keys"
    );
    // NOTE: the `wait_until` above IS the assertion — it polls until both admin
    // grants are present. Do NOT re-read a fresh instantaneous snapshot and
    // assert on it: the relay's member set converges (and can transiently flap)
    // during initial provisioning, so a bare re-read races that convergence
    // (TOCTOU) even after the wait just observed the grant.

    let store = Store::open(&home.store_path()).unwrap();
    let pubkey = pubkey_for_harness_session(&store, "claude-code", &sid).unwrap();
    let session = store.get_session(&pubkey).unwrap();
    assert!(
        session.as_ref().is_some_and(|rec| rec.is_running()),
        "successful readiness should leave a live session row; session={session:?}; daemon_log={}",
        test_log(&home, "daemon.log")
    );

    stop_daemon(&home);
}

#[test]
fn generated_management_key_self_grants_on_existing_user_owned_channel() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec_without_backend_key(&home, false);
    let channel = unique_session("existing-channel");
    let cwd = home.dir.path().join("existing-work");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut channels = serde_json::Map::new();
    channels.insert(
        channel.clone(),
        serde_json::Value::String(cwd.to_string_lossy().to_string()),
    );
    std::fs::write(
        home.dir.path().join("workspaces.json"),
        serde_json::to_string(&serde_json::Value::Object(channels)).unwrap(),
    )
    .unwrap();
    let sid = unique_session("sess-selfgrant");

    rt().block_on(async {
        precreate_channel_group_as_user(&channel).await;
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({"agent": "coder", "harness_session": &sid, "cwd": &cwd}),
                "claude-code",
            ),
        )
        .await
        .expect("session_start should self-grant generated management key");
    });

    assert!(
        wait_until(Duration::from_secs(25), || {
            std::fs::read_to_string(home.dir.path().join("config.json"))
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|cfg| cfg["mosaicoPrivateKey"].as_str().map(str::to_string))
                .is_some()
        }),
        "background readiness should generate mosaicoPrivateKey"
    );
    let cfg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.dir.path().join("config.json")).unwrap(),
    )
    .unwrap();
    let backend_pk = pubkey_of(cfg["mosaicoPrivateKey"].as_str().unwrap());
    if !wait_until(Duration::from_secs(25), || {
        refresh_channel_members(&format!("#{channel}"));
        let members = Store::open(&home.store_path())
            .and_then(|store| store.list_channel_members(&channel))
            .unwrap_or_default();
        members
            .iter()
            .any(|m| m.pubkey == backend_pk && m.role == "admin")
    }) {
        panic!(
            "background readiness should self-grant generated management key; daemon_log={}; group_log={}",
            test_log(&home, "daemon.log"),
            test_log(&home, "logs/group-mgmt.log")
        );
    }
    // The `wait_until` above IS the assertion (it polls until the self-grant
    // materializes). A bare re-read here would race the relay's converging member
    // snapshot (TOCTOU): the wait can observe the admin grant and a fresh read a
    // moment later miss it while the set is still flapping toward convergence.

    stop_daemon(&home);
}

fn test_log(home: &Home, rel: &str) -> String {
    std::fs::read_to_string(home.dir.path().join(rel)).unwrap_or_else(|e| format!("<{e}>"))
}

#[test]
fn session_start_schedules_unverified_channel_work_without_blocking() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "coder", "harness_session": "sess-nogrp", "cwd": "/tmp"}), "claude-code"),
        )
        .await
        .expect("session_start should register locally without waiting on relay readiness");
    });

    // Relay readiness is daemon-side work now; session_start itself is the local
    // registration edge and must not block the harness on relay proof.
    let store = Store::open(&home.store_path()).unwrap();
    let pubkey = pubkey_for_harness_session(&store, "claude-code", "sess-nogrp").unwrap();
    assert!(
        store
            .get_session(&pubkey)
            .unwrap()
            .map(|rec| rec.is_running())
            .unwrap_or(false),
        "session_start should leave a live local session row"
    );

    stop_daemon(&home);
}

/// Regression: a duplicate session-start fired by the offline-agent-mention
/// handler with a different requested launch channel must not replace the
/// running session's joined-route set or add a spurious join. Before the fix,
/// stale launch input rewrote the session scope and delivered inbox messages
/// from a channel the session had never explicitly joined.
#[test]
fn session_reassert_with_wrong_channel_does_not_corrupt_joined_routes() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-reassert");
    let real_channel = unique_session("nostr-multi-platform");
    let stale_channel = unique_session("backlog");

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // First start: the engine joins the requested launch channel.
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "codex",
                    "harness_session": &sid,
                    "cwd": "/tmp",
                    "channel": real_channel,
                    "watch_pid": std::process::id()
                }),
                "claude-code",
            ),
        )
        .await
        .expect("first session_start");
    });

    // Read the resolved private route from the store for the internal assertion.
    let lookup_store = Store::open(&home.store_path()).unwrap();
    let pubkey = pubkey_for_harness_session(&lookup_store, "claude-code", &sid).unwrap();
    let stored_real_channel = only_session_route(&lookup_store, &pubkey);
    assert!(
        !stored_real_channel.is_empty(),
        "initial channel must be resolved and stored"
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Re-assert from a duplicate spawn with a different requested launch
        // channel while the engine is already live.
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": "codex",
                    "harness_session": &sid,
                    "cwd": "/tmp",
                    "channel": stale_channel,
                    "watch_pid": std::process::id()
                }),
                "claude-code",
            ),
        )
        .await
        .expect("re-assert session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    // The session must only be joined to the real channel -- exactly one entry.
    // A spurious re-assert with a different channel used to add a second passive
    // join for the stale channel, leaving two rows in session_channels.
    let joined = store
        .list_session_routes(&pubkey)
        .expect("list_session_routes");
    assert_eq!(
        joined.len(),
        1,
        "session must have exactly one channel join after a re-assert; got {:?}",
        joined
    );
    assert_eq!(
        joined[0].0, stored_real_channel,
        "the only joined channel must be the real (original) channel"
    );

    stop_daemon(&home);
}
