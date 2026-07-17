use crate::daemon_harness::*;
use mosaico::daemon::client::Client;

#[test]
fn hooks_fail_open_without_spawning_daemon() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    assert!(
        !home.sock().exists(),
        "precondition: daemon must be stopped"
    );
    // Keep this a hook-path latency assertion rather than a macOS debug-binary
    // cold-page-fault assertion. NMP materially increases the debug executable;
    // one non-hook process warms the image without creating the daemon.
    let warm = run_cli(&home, &["--help"]);
    assert!(warm.status.success(), "debug binary warmup failed");
    assert!(!home.sock().exists(), "warmup must not spawn the daemon");

    let payload = r#"{"cwd":"/tmp","session_id":"s-no-daemon"}"#;
    let cases = [
        ("claude-code", "session-start", payload),
        ("claude-code", "session-end", payload),
        ("claude-code", "user-prompt-submit", payload),
        ("claude-code", "post-tool-use", payload),
        ("claude-code", "stop", payload),
        ("codex", "session-start", payload),
        ("codex", "user-prompt-submit", payload),
        ("codex", "post-tool-use", payload),
        ("codex", "stop", payload),
        ("opencode", "session-start", payload),
        ("opencode", "session-end", payload),
        ("opencode", "user-prompt-submit", payload),
        ("opencode", "post-tool-use", payload),
        ("opencode", "stop", payload),
        ("grok", "session-start", payload),
        ("grok", "session-end", payload),
        ("grok", "user-prompt-submit", payload),
        ("grok", "post-tool-use", payload),
        ("grok", "stop", payload),
    ];
    let started = std::time::Instant::now();
    for (host, hook, payload) in cases {
        let hook_started = std::time::Instant::now();
        let out = run_cli_stdin(&home, &["harness", "hook", host, "--type", hook], payload);
        assert!(
            out.status.success(),
            "{host} {hook} failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            hook_started.elapsed() < std::time::Duration::from_secs(2),
            "{host} {hook} should fail open quickly, elapsed {:?}",
            hook_started.elapsed()
        );
        assert!(
            !home.sock().exists(),
            "{host} {hook} must not spawn the daemon from the hook path"
        );
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "no-daemon hooks should fail open quickly, elapsed {:?}",
        started.elapsed()
    );
}

#[test]
fn hook_serving_rpcs_return_while_relay_is_wedged() {
    // Leave one second of headroom beneath the production five-second hook cap.
    const DEADLINE: std::time::Duration = std::time::Duration::from_secs(4);
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let relay = WedgeRelay::start();
    let home = Home::with_wedged_relay(&relay.url);

    let hook_started = std::time::Instant::now();
    let out = run_cli_stdin_with_env(
        &home,
        &["harness", "hook", "claude-code", "--type", "session-start"],
        r#"{"cwd":"/tmp","session_id":"wedged-hook-start"}"#,
        &[("MOSAICO_OBSERVED_HARNESS", "claude-code")],
    );
    assert!(
        out.status.success(),
        "session-start hook failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        hook_started.elapsed() < DEADLINE,
        "session-start hook exceeded latency budget: {:?}",
        hook_started.elapsed()
    );

    let old_pubkey = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect to daemon");
        let started = tokio::time::timeout(
            DEADLINE,
            client.call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "claude-code",
                        "harness_session": "wedged-old",
                        "cwd": "/tmp"
                    }),
                    "claude-code",
                ),
            ),
        )
        .await
        .expect("fresh session_start exceeded hook latency budget")
        .expect("fresh session_start");
        started["pubkey"].as_str().expect("old pubkey").to_string()
    });

    rt().block_on(async {
        let mut client = Client::connect_or_spawn()
            .await
            .expect("reconnect to daemon");
        tokio::time::timeout(
            DEADLINE,
            client.call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "codex",
                        "harness_session": "wedged-replacement",
                        "reclaimed_pubkey": &old_pubkey,
                        "cwd": "/tmp"
                    }),
                    "codex",
                ),
            ),
        )
        .await
        .expect("reclaimed-profile session_start exceeded hook latency budget")
        .expect("reclaimed-profile session_start");
    });

    let hook_started = std::time::Instant::now();
    let out = run_cli_stdin_with_env(
        &home,
        &[
            "harness",
            "hook",
            "claude-code",
            "--type",
            "user-prompt-submit",
        ],
        r#"{"cwd":"/tmp","session_id":"wedged-old","prompt":"work"}"#,
        &[("MOSAICO_OBSERVED_HARNESS", "claude-code")],
    );
    assert!(
        out.status.success(),
        "turn hook failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        hook_started.elapsed() < DEADLINE,
        "turn hook exceeded latency budget: {:?}",
        hook_started.elapsed()
    );

    let hook_started = std::time::Instant::now();
    let out = run_cli_stdin(
        &home,
        &["harness", "hook", "claude-code", "--type", "post-tool-use"],
        r#"{"cwd":"/tmp","session_id":"wedged-old"}"#,
    );
    assert!(
        out.status.success(),
        "post-tool-use hook failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        hook_started.elapsed() < DEADLINE,
        "post-tool-use hook exceeded latency budget: {:?}",
        hook_started.elapsed()
    );

    for hook in ["stop", "session-end"] {
        let started = std::time::Instant::now();
        let out = run_cli_stdin(
            &home,
            &["harness", "hook", "claude-code", "--type", hook],
            r#"{"cwd":"/tmp","session_id":"wedged-old"}"#,
        );
        assert!(
            out.status.success(),
            "{hook} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            started.elapsed() < DEADLINE,
            "{hook} exceeded latency budget: {:?}",
            started.elapsed()
        );
    }

    let store = mosaico::state::Store::open(&home.store_path()).expect("open store");
    assert!(
        store
            .get_session(&old_pubkey)
            .expect("session lookup")
            .is_some_and(|session| !session.alive),
        "session-end store projection must survive a wedged relay"
    );
    stop_daemon(&home);
}
