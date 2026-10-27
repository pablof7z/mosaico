//! The real binary's blocking client path, driven the way harness hooks run it.
use super::*;

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
    // The opencode plugin names its own process; the observed harness is given
    // explicitly so the result never depends on which harness (if any) is an
    // ancestor of the test runner.
    let payload = format!(r#"{{"cwd":"/tmp","pid":{}}}"#, std::process::id());
    let out = run_cli_stdin_with_env(
        &home,
        &["harness", "hook", "opencode", "--type", "session-start"],
        &payload,
        &[("MOSAICO_OBSERVED_HARNESS", "opencode")],
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
    assert_eq!(first_hook["stdin"]["raw"], payload);
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
