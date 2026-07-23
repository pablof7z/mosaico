use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;

#[path = "process/hooks.rs"]
mod hooks;
#[path = "process/statusline.rs"]
mod statusline;
#[path = "process/who.rs"]
mod who;

#[test]
fn sixteen_concurrent_writers_no_corruption_single_writer() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_channel("mosaico", "mosaico", "", "", 1)
        .unwrap();

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
                        c.call("who", serde_json::json!({"all": true}))
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
fn cli_subprocess_blocking_path_session_start_and_who() {
    // The session/turn lifecycle is driven only through `harness hook` now (no
    // bare verbs). Run the real binary the way the harnesses do — payload on
    // stdin — and assert the blocking client + renderer behave. This also
    // exercises the opencode-only branch where no native locator is supplied.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call("ping", serde_json::json!({})).await.expect("ping");
    });

    // Session-start with no native id: the daemon allocates the pubkey identity,
    // while the hook remains silent because identities are not runtime output.
    let out = run_cli_stdin(
        &home,
        &["harness", "hook", "opencode", "--type", "session-start"],
        r#"{"cwd":"/tmp"}"#,
    );
    assert!(
        out.status.success(),
        "session-start failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "session-start leaked identity output: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let mut sid = None;
    assert!(
        wait_until(std::time::Duration::from_secs(5), || {
            sid = Store::open(&home.store_path()).ok().and_then(|store| {
                store
                    .list_running_sessions()
                    .ok()?
                    .into_iter()
                    .find(|session| session.agent_slug == "opencode")
                    .map(|session| session.pubkey)
            });
            sid.is_some()
        }),
        "session-start did not register an opencode pubkey"
    );
    let sid = sid.unwrap();
    // Forensics logs are now scoped to per-session dirs. The session-start hook
    // payload has no session_id key (opencode with no id), so it lands in _unscoped.
    let hook_log_path = home
        .dir
        .path()
        .join("sessions")
        .join("_unscoped")
        .join("hook-calls.jsonl");
    let hook_log = std::fs::read_to_string(&hook_log_path).expect("hook forensic log");
    let first_hook: serde_json::Value =
        serde_json::from_str(hook_log.lines().next().expect("hook log line"))
            .expect("hook log json");
    assert_eq!(first_hook["hook"]["host"], "opencode");
    assert_eq!(first_hook["hook"]["type"], "session-start");
    assert_eq!(first_hook["stdin"]["raw"], r#"{"cwd":"/tmp"}"#);
    assert_eq!(
        first_hook["process"]["argv"][0],
        bin().display().to_string()
    );
    assert!(
        first_hook["parent_chain"].as_array().is_some(),
        "parent chain should be captured"
    );

    // who --all-workspaces shows the agent (blocking client + real renderer).
    assert!(
        wait_until(std::time::Duration::from_secs(5), || {
            let out = run_cli(&home, &["who", "--all-workspaces"]);
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains("opencode")
        }),
        "who output missing agent: {}",
        String::from_utf8_lossy(&run_cli(&home, &["who", "--all-workspaces"]).stdout)
    );

    // turn end (stop hook) is a sync blocking write — must succeed, print nothing.
    let out = run_cli_stdin(
        &home,
        &["harness", "hook", "opencode", "--type", "stop"],
        &format!(r#"{{"session_id":"{sid}"}}"#),
    );
    assert!(
        out.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // session-end is a hook: it should exit cleanly and mark the session dead,
    // without relying on user-facing confirmation text.
    let out = run_cli_stdin_with_env(
        &home,
        &["harness", "hook", "opencode", "--type", "session-end"],
        &format!(r#"{{"session_id":"{sid}"}}"#),
        &[("MOSAICO_PUBKEY", &sid)],
    );
    assert!(out.status.success());
    assert!(wait_until(std::time::Duration::from_secs(5), || {
        Store::open(&home.store_path())
            .and_then(|store| store.get_session(&sid))
            .unwrap_or(None)
            .map(|rec| !rec.is_running())
            .unwrap_or(false)
    }));

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

#[test]
fn version_skew_client_detects_and_respawns() {
    // A daemon spawned at protocol 1, then a NEWER client (protocol 2) running
    // the real `connect_or_spawn` must detect the skew, make the old daemon
    // exit, and respawn the (now "newer") daemon — transparently succeeding.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    // Start a daemon pinned to protocol 1 via a normal subprocess CLI call.
    let out = run_cli_proto(&home, &["who", "--all-workspaces"], Some("1"));
    assert!(
        out.status.success(),
        "proto-1 who failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.sock().exists(), "daemon should be up at proto 1");

    // A "newer" client (protocol 2): connect_or_spawn must re-exec the daemon
    // and the call must still succeed.
    let out = run_cli_proto(&home, &["who", "--all-workspaces"], Some("2"));
    assert!(
        out.status.success(),
        "proto-2 client failed to respawn+succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

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

fn run_cli_proto(home: &Home, args: &[&str], proto: Option<&str>) -> std::process::Output {
    let mut cmd = std::process::Command::new(bin());
    cmd.args(args)
        .env_remove("MOSAICO_AGENT")
        .env_remove("MOSAICO_PUBKEY")
        .env_remove("MOSAICO_PTY_SESSION")
        .env_remove("MOSAICO_PTY_SOCKET")
        .env_remove("MOSAICO_CHANNEL")
        .env_remove("MOSAICO_EPHEMERAL")
        .env("MOSAICO_HOME", home.dir.path())
        .env("MOSAICO_CONFIG", home.dir.path().join("config.json"))
        .env("MOSAICO_BIN", bin())
        .env("MOSAICO_DAEMON_GRACE_S", "30");
    if let Some(p) = proto {
        cmd.env("MOSAICO_PROTOCOL", p);
    }
    cmd.output().expect("run mosaico")
}
