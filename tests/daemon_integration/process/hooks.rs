use crate::daemon_harness::*;

#[test]
fn hooks_fail_open_without_spawning_daemon() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    assert!(
        !home.sock().exists(),
        "precondition: daemon must be stopped"
    );

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
