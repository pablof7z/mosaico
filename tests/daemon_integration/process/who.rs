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
        "both workspace roots must be delivered by Nostr runtime before renderer sessions start"
    );

    // The opencode plugin names its own process; the observed harness is given
    // explicitly so the result never depends on the test runner's ancestors.
    let observed = [("MOSAICO_OBSERVED_HARNESS", "opencode")];
    let pid = std::process::id();
    let out = run_cli_stdin_with_env(
        &home,
        &["harness", "hook", "opencode", "--type", "session-start"],
        &format!(r#"{{"cwd":"/tmp","session_id":"sid-tmp","pid":{pid}}}"#),
        &observed,
    );
    assert!(out.status.success(), "session-start (tmp) failed");

    let payload = serde_json::json!({
        "cwd": second_dir.path().display().to_string(),
        "session_id": "sid-proj2",
        "pid": pid,
    })
    .to_string();
    let out = run_cli_stdin_with_env_in_dir(
        &home,
        &["harness", "hook", "opencode", "--type", "session-start"],
        &payload,
        &observed,
        second_dir.path(),
    );
    assert!(out.status.success(), "session-start (proj2) failed");

    let handle_for = |harness_session: &str| {
        let store = Store::open(&home.store_path()).unwrap();
        let pubkey = pubkey_for_harness_session(&store, "opencode", harness_session)
            .unwrap_or_else(|| panic!("session-start did not register {harness_session}"));
        store
            .session_identity(&pubkey)
            .unwrap()
            .expect("session identity")
            .display_slug()
    };
    let (tmp_handle, proj2_handle) = (handle_for("sid-tmp"), handle_for("sid-proj2"));

    // Presence is the heartbeat: each session is listed in its own channel
    // block once its first status lands.
    let mut out = None;
    let ready = wait_until(std::time::Duration::from_secs(25), || {
        let candidate = run_cli(&home, &["who", "--all-workspaces"]);
        let who = String::from_utf8_lossy(&candidate.stdout);
        let is_ready = candidate.status.success()
            && channel_block(&who, "#tmp").contains(&format!("@{tmp_handle} "))
            && channel_block(&who, "#proj2").contains(&format!("@{proj2_handle} "))
            && !who.contains("no relay-backed channel metadata");
        out = Some(candidate);
        is_ready
    });
    let out = out.expect("who --all-workspaces was not attempted");
    assert!(
        ready,
        "who --all-workspaces did not list @{tmp_handle} under #tmp and @{proj2_handle} \
         under #proj2:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let who = String::from_utf8_lossy(&out.stdout);
    assert!(
        !who.contains("| Agent | Host | Title | Status |"),
        "who --all-workspaces still uses the old markdown table renderer:\n{who}"
    );

    stop_daemon(&home);
}

/// The rendered block for one channel: its header line up to the next
/// top-level channel header.
fn channel_block<'a>(who: &'a str, header: &str) -> &'a str {
    let Some(start) = who
        .match_indices(header)
        .map(|(index, _)| index)
        .find(|&index| {
            (index == 0 || who[..index].ends_with('\n'))
                && who[index + header.len()..].starts_with('\n')
        })
    else {
        return "";
    };
    let body = &who[start + header.len()..];
    let end = body.find("\n#").map_or(body.len(), |index| index + 1);
    &body[..end]
}
