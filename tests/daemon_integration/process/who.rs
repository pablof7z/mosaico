use super::*;

#[test]
fn all_workspaces_uses_unified_fabric_render_not_old_table() {
    // `who --all-workspaces` must render through the same fabric pipeline as
    // single-channel `who`, with one channel block per workspace.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    crate::channels::write_config(&home, false);

    let second_dir = tempfile::tempdir().unwrap();
    let workspace_map = serde_json::json!({ "tmp": "/tmp", "proj2": second_dir.path() });
    std::fs::write(
        home.dir.path().join("workspaces.json"),
        serde_json::to_string(&workspace_map).unwrap(),
    )
    .unwrap();
    crate::channels::initialize_workspace_root("tmp", "/tmp");
    crate::channels::initialize_workspace_root("proj2", second_dir.path().to_str().unwrap());
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            observed_channel_members("#tmp").is_some()
                && observed_channel_members("#proj2").is_some()
        }),
        "both workspace roots must be delivered by NMP before renderer sessions start"
    );

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

    let mut out = None;
    let ready = wait_until(std::time::Duration::from_secs(25), || {
        let candidate = run_cli(&home, &["who", "--all-workspaces"]);
        let who = String::from_utf8_lossy(&candidate.stdout);
        let is_ready = candidate.status.success()
            && who.contains("opencode")
            && who.contains("tmp")
            && who.contains("proj2")
            && !who.contains("no relay-backed channel metadata");
        out = Some(candidate);
        is_ready
    });
    let out = out.expect("who --all-workspaces was not attempted");
    assert!(
        ready,
        "who --all-workspaces did not observe relay-backed channels:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let who = String::from_utf8_lossy(&out.stdout);
    assert!(
        !who.contains("| Agent | Host | Title | Status |"),
        "who --all-workspaces still uses the old markdown table renderer:\n{who}"
    );
    assert!(
        who.contains("opencode"),
        "who --all-workspaces missing agent:\n{who}"
    );
    assert!(
        who.contains("tmp") && who.contains("proj2"),
        "who --all-workspaces missing a channel block:\n{who}"
    );

    stop_daemon(&home);
}
