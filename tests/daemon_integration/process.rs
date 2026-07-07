use crate::daemon_harness::*;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[path = "process/statusline.rs"]
mod statusline;

#[test]
fn sixteen_concurrent_writers_no_corruption_single_writer() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    // Start one session, then 16 concurrent clients hammer write-RPCs
    // (turn_start/turn_end flip turn state; this is the corruption repro path,
    // now serialized through the ONE daemon writer).
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "s-load", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
    });

    let n = 16;
    let iters = 25;
    let handles: Vec<_> = (0..n)
        .map(|_| {
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let mut c = Client::connect_or_spawn().await.expect("connect");
                    for _ in 0..iters {
                        c.call("turn_start", serde_json::json!({"session": "s-load"}))
                            .await
                            .expect("turn_start");
                        c.call("turn_end", serde_json::json!({"session": "s-load"}))
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
    // exercises the opencode-only "no session id supplied → generate + print"
    // branch.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    // session-start with no id: the daemon generates one, the hook prints it.
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
    // The opencode session-start hook echoes the daemon-minted canonical id as
    // JSON ({"session_id":"te-..."}); the plugin parses it.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();
    let sid = serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|v| v["session_id"].as_str().map(str::to_string))
        .unwrap_or_else(|| trimmed.to_string());
    assert!(
        !sid.is_empty(),
        "session-start printed no generated session id"
    );
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

    // who --all-projects shows the agent (blocking client + real renderer).
    let out = run_cli(&home, &["who", "--all-projects"]);
    assert!(
        out.status.success(),
        "who failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let who = String::from_utf8_lossy(&out.stdout);
    assert!(who.contains("opencode"), "who output missing agent: {who}");

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

    // session-end prints the confirmation to stderr.
    let out = run_cli_stdin(
        &home,
        &["harness", "hook", "opencode", "--type", "session-end"],
        &format!(r#"{{"session_id":"{sid}"}}"#),
    );
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ended"));

    stop_daemon(&home);
}

#[test]
fn invalid_cli_invocation_writes_command_log_only_when_enabled() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let args = &[
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
            tenex_edge::command_forensics::COMMAND_CALL_LOG_ENV,
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
    assert_eq!(received["schema"], "tenex-edge.command-call.v1");
    assert_eq!(received["command"]["subcommand"], "send-this-message");
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

    let out = run_cli_stdin(
        &home,
        &[
            "harness",
            "hook",
            "claude-code",
            "--type",
            "user-prompt-submit",
        ],
        r#"{"session_id":"revive-claude","cwd":"/tmp","prompt":"hello"}"#,
    );
    assert!(
        out.status.success(),
        "user-prompt-submit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("revive-claude")
        .unwrap()
        .expect("revived session row");
    assert!(rec.alive);
    assert_eq!(rec.agent_slug, "claude");

    stop_daemon(&home);
}

#[test]
fn who_all_projects_uses_unified_fabric_render_not_old_table() {
    // Regression for the divergence the user flagged live: `who --all-projects`
    // must render through the SAME fabric pipeline as single-project `who`
    // (one project block per root channel), not the old flat markdown table.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    // Register a second project alongside the default "tmp" -> /tmp mapping.
    let second_dir = tempfile::tempdir().unwrap();
    let projects_map = serde_json::json!({ "tmp": "/tmp", "proj2": second_dir.path() });
    std::fs::write(
        home.dir.path().join("projects.json"),
        serde_json::to_string(&projects_map).unwrap(),
    )
    .unwrap();

    let out = run_cli_stdin(
        &home,
        &["harness", "hook", "opencode", "--type", "session-start"],
        r#"{"cwd":"/tmp","session_id":"sid-tmp"}"#,
    );
    assert!(out.status.success(), "session-start (tmp) failed");

    let payload = serde_json::json!({
        "cwd": second_dir.path().display().to_string(),
        "session_id": "sid-proj2",
    })
    .to_string();
    let out = run_cli_stdin_with_env_in_dir(
        &home,
        &["harness", "hook", "opencode", "--type", "session-start"],
        &payload,
        &[],
        second_dir.path(),
    );
    assert!(out.status.success(), "session-start (proj2) failed");

    let out = run_cli(&home, &["who", "--all-projects"]);
    assert!(
        out.status.success(),
        "who --all-projects failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let who = String::from_utf8_lossy(&out.stdout);
    assert!(
        !who.contains("| Agent | Host | Title | Status |"),
        "who --all-projects still uses the old markdown table renderer:\n{who}"
    );
    assert!(
        who.contains("opencode"),
        "who --all-projects missing agent:\n{who}"
    );
    assert!(
        who.contains("tmp") && who.contains("proj2"),
        "who --all-projects missing a project block:\n{who}"
    );

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
    let out = run_cli_proto(&home, &["who", "--all-projects"], Some("1"));
    assert!(
        out.status.success(),
        "proto-1 who failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.sock().exists(), "daemon should be up at proto 1");

    // A "newer" client (protocol 2): connect_or_spawn must re-exec the daemon
    // and the call must still succeed.
    let out = run_cli_proto(&home, &["who", "--all-projects"], Some("2"));
    assert!(
        out.status.success(),
        "proto-2 client failed to respawn+succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    stop_daemon(&home);
}

/// Regression for the identity-normalization bug: hooks send the HARNESS id, but
/// the daemon mints a CANONICAL id and stores the harness id as an alias. The turn
/// transitions must resolve harness→canonical or they silently update zero rows
/// (the canonical aggregate would stay idle/untitled forever for claude/codex).
/// Drive the real RPC path with the harness id and assert the CANONICAL row moved.
#[test]
fn turn_lifecycle_by_harness_alias_drives_canonical_row() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let canonical = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // claude-style: harness supplies its own session id.
        let resp = c
            .call(
                "session_start",
                serde_json::json!({"agent":"coder","session_id":"harness-xyz","cwd":"/tmp"}),
            )
            .await
            .unwrap();
        let canonical = resp["session_id"].as_str().unwrap().to_string();

        // The hook drives turns by the HARNESS id, never the canonical id.
        c.call("turn_start", serde_json::json!({"session":"harness-xyz"}))
            .await
            .expect("turn_start");
        canonical
    });

    let store = Store::open(&home.store_path()).unwrap();

    // The daemon must mint a canonical id distinct from the harness id...
    assert_ne!(
        canonical, "harness-xyz",
        "daemon must MINT a canonical id; the harness id is only an alias"
    );
    // ...and the harness id must be an ALIAS that resolves to the canonical row,
    // not a second canonical session. (`get_session` is alias-resolving, so the
    // raw harness id returns the SAME canonical row — proving it's an alias.
    // `local_session_snapshot` → `get_session`.)
    assert_eq!(
        store
            .get_session("harness-xyz")
            .unwrap()
            .expect("harness id resolves to a session")
            .session_id,
        canonical,
        "harness id must be an alias onto the canonical row, not a second row"
    );

    // turn_start via the harness alias must have moved the CANONICAL row: working,
    // with a turn start timestamp. (The pre-fix bug left it idle. The new schema
    // tracks `working`/`turn_started_at`; there is no turn_id counter.)
    let started = store
        .get_session(&canonical)
        .unwrap()
        .expect("canonical session row");
    assert!(
        started.working,
        "turn_start via harness alias must set the CANONICAL row working"
    );
    assert!(
        started.turn_started_at > 0,
        "turn_start via harness alias must stamp the CANONICAL turn start time"
    );

    // turn_end via the harness alias must close the CANONICAL turn.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call("turn_end", serde_json::json!({"session":"harness-xyz"}))
            .await
            .expect("turn_end");
    });
    let ended = store
        .get_session(&canonical)
        .unwrap()
        .expect("canonical session row");
    assert!(
        !ended.working,
        "turn_end via harness alias must clear working on the CANONICAL row"
    );

    stop_daemon(&home);
}

fn run_cli_proto(home: &Home, args: &[&str], proto: Option<&str>) -> std::process::Output {
    let mut cmd = std::process::Command::new(bin());
    cmd.args(args)
        .env_remove("TENEX_EDGE_AGENT")
        .env_remove("TENEX_EDGE_AGENT_FALLBACK")
        .env_remove("TENEX_EDGE_SESSION")
        .env_remove("TENEX_EDGE_PTY_SESSION")
        .env_remove("TENEX_EDGE_PTY_SOCKET")
        .env_remove("TENEX_EDGE_CHANNEL")
        .env_remove("TENEX_EDGE_EPHEMERAL")
        .env("TENEX_EDGE_HOME", home.dir.path())
        .env("TENEX_CONFIG", home.dir.path().join("config.json"))
        .env("TENEX_EDGE_BIN", bin())
        .env("TENEX_EDGE_DAEMON_GRACE_S", "30");
    if let Some(p) = proto {
        cmd.env("TENEX_EDGE_PROTOCOL", p);
    }
    cmd.output().expect("run tenex-edge")
}
