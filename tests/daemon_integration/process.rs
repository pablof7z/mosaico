use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;

#[path = "process/blocking_cli.rs"]
mod blocking_cli;
#[path = "process/cross_project_guard.rs"]
mod cross_project_guard;
#[path = "process/hooks.rs"]
mod hooks;
#[path = "process/profile_retirement.rs"]
mod profile_retirement;
#[path = "process/version_skew.rs"]
mod version_skew;
#[path = "process/who.rs"]
mod who;

#[test]
fn sixteen_concurrent_writers_no_corruption_single_writer() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    crate::channels::write_config(&home, false);
    crate::channels::initialize_workspace_root("tmp", "/tmp");
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            observed_channel_members("#tmp").is_some()
        }),
        "concurrent RPC fixture requires a Nostr runtime-observed #tmp root"
    );

    // Start one session, then 16 concurrent clients hammer write-RPCs
    // (turn_start/turn_end flip turn state; this is the corruption repro path,
    // now serialized through the ONE daemon writer).
    let pubkey = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let started = c
            .call(
                "session_start",
                hook_session_start(serde_json::json!({"agent": "coder", "harness_session": "s-load", "cwd": "/tmp"}), "claude-code"),
            )
            .await
            .unwrap();
        started["pubkey"].as_str().unwrap().to_string()
    });
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            observed_channel_has_role("#tmp", &pubkey, "member")
        }),
        "concurrent RPC load must start only after Nostr runtime delivers the session roster"
    );

    let n = 16;
    let iters = 25;
    let handles: Vec<_> = (0..n)
        .map(|_| {
            let pubkey = pubkey.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let mut c = Client::connect_or_spawn().await.expect("connect");
                    for _ in 0..iters {
                        c.call("turn_start", serde_json::json!({"session": &pubkey}))
                            .await
                            .expect("turn_start");
                        c.call("turn_end", serde_json::json!({"session": &pubkey}))
                            .await
                            .expect("turn_end");
                        c.call("who", serde_json::json!({"workspace": "tmp"}))
                            .await
                            .expect("who");
                    }
                });
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    // Single-writer by construction: exactly one daemon socket. And the db must
    // pass an integrity check (the corruption we recovered from would fail here).
    assert!(home.sock().exists(), "one daemon should still be listening");
    let store = Store::open(&home.store_path()).unwrap();
    let integrity = store.integrity_check().expect("integrity_check");
    assert_eq!(
        integrity, "ok",
        "state.db integrity check failed: {integrity}"
    );

    stop_daemon(&home);
}

#[test]
fn invalid_cli_invocation_writes_command_log_only_when_enabled() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let args = &[
        "agents",
        "send-this-message",
        "--session",
        "hallucinated-session",
        "hello",
    ];
    let out = run_cli(&home, args);
    assert!(
        !out.status.success(),
        "hallucinated command unexpectedly succeeded"
    );
    assert!(
        !home
            .dir
            .path()
            .join("sessions")
            .join("hallucinated-session")
            .join("command-calls.jsonl")
            .exists(),
        "default CLI execution must not write command forensic logs"
    );

    let command_log_path = home.dir.path().join("command-calls.jsonl");
    let out = run_cli_with_env(
        &home,
        args,
        &[(
            mosaico::command_forensics::COMMAND_CALL_LOG_ENV,
            command_log_path.to_str().unwrap(),
        )],
    );
    assert!(
        !out.status.success(),
        "hallucinated command unexpectedly succeeded"
    );

    let command_log = std::fs::read_to_string(&command_log_path).expect("command forensic log");
    let records: Vec<serde_json::Value> = command_log
        .lines()
        .map(|line| serde_json::from_str(line).expect("command log json"))
        .collect();
    let received = records
        .iter()
        .find(|v| v["phase"] == "received")
        .expect("received record");
    assert_eq!(received["schema"], "mosaico.command-call.v1");
    assert_eq!(received["command"]["subcommand"], "agents");
    assert_eq!(
        received["command"]["explicit_session"],
        "hallucinated-session"
    );

    let finished = records
        .iter()
        .find(|v| v["phase"] == "finished")
        .expect("finished record");
    assert_eq!(finished["result"]["ok"], false);
    assert_eq!(finished["result"]["kind"], "InvalidSubcommand");
    assert!(
        finished["result"]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("unrecognized subcommand"),
        "unexpected clap error: {finished}"
    );
}

#[test]
fn claude_user_prompt_submit_reasserts_missing_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call("ping", serde_json::json!({})).await.expect("ping");
    });

    let out = run_cli_stdin_with_env(
        &home,
        &[
            "harness",
            "hook",
            "claude-code",
            "--type",
            "user-prompt-submit",
        ],
        r#"{"session_id":"revive-claude","cwd":"/tmp","prompt":"hello"}"#,
        &[("MOSAICO_OBSERVED_HARNESS", "claude-code")],
    );
    assert!(
        out.status.success(),
        "user-prompt-submit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let store = Store::open(&home.store_path()).unwrap();
    let pubkey = store
        .resolve_pubkey_by_locator("claude-code", "native_resume", "revive-claude")
        .unwrap()
        .expect("revived session locator");
    let rec = store
        .get_session(&pubkey)
        .unwrap()
        .expect("revived session row");
    assert!(rec.is_running());
    assert_eq!(rec.agent_slug, "claude");

    stop_daemon(&home);
}

/// Harness-owned ids remain typed locators; lifecycle RPCs operate on the
/// resolved public session identity.
#[test]
fn turn_lifecycle_drives_pubkey_row_resolved_from_harness_locator() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let pubkey = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // claude-style: harness supplies its own session id.
        let resp = c
            .call(
                "session_start",
                hook_session_start(serde_json::json!({"agent":"coder","harness_session":"harness-xyz","cwd":"/tmp"}), "claude-code"),
            )
            .await
            .unwrap();
        let pubkey = resp["pubkey"].as_str().unwrap().to_string();

        c.call(
            "turn_start",
            serde_json::json!({
                "harness_session": "harness-xyz",
                "harness": "claude-code"
            }),
        )
        .await
        .expect("turn_start");
        pubkey
    });

    let store = Store::open(&home.store_path()).unwrap();
    assert_eq!(
        store
            .resolve_pubkey_by_locator("claude-code", "native_resume", "harness-xyz")
            .unwrap()
            .as_deref(),
        Some(pubkey.as_str())
    );

    let started = store.get_session(&pubkey).unwrap().expect("session row");
    assert!(
        started.is_working(),
        "turn_start must set the pubkey row working"
    );
    assert!(
        started.turn_started_at > 0,
        "turn_start must stamp the pubkey row turn start time"
    );
    store
        .apply_session_presentation_edge(
            &pubkey,
            started.runtime_generation,
            1,
            mosaico::state::PresentationState::Headless,
            started.turn_started_at,
        )
        .unwrap();
    store
        .enqueue_inbox(
            "native-turn-injected-event",
            &pubkey,
            "operator",
            "mosaico",
            "keep working",
            started.turn_started_at,
        )
        .unwrap();
    store
        .claim_pending_for_pubkey(&pubkey, started.turn_started_at)
        .unwrap();
    store
        .mark_injected_for_echo(&["native-turn-injected-event".into()], &pubkey, 0)
        .unwrap();

    // turn_end via the harness alias must close the CANONICAL turn.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "turn_end",
            serde_json::json!({
                "harness_session": "harness-xyz",
                "harness": "claude-code"
            }),
        )
        .await
        .expect("turn_end");
    });
    let ended = store
        .get_session(&pubkey)
        .unwrap()
        .expect("pubkey-owned session row");
    assert!(
        !ended.is_working(),
        "turn_end must clear working on the pubkey row"
    );
    assert!(ended.idle_since >= started.turn_started_at);
    assert_eq!(ended.idle_deadline, ended.idle_since + 10 * 60);
    assert!(
        store.injected_for_pubkey(&pubkey).unwrap().is_empty(),
        "the completed turn must release injected-message eviction fences"
    );

    stop_daemon(&home);
}
