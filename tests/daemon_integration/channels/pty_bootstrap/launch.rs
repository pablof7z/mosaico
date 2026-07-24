use super::*;

struct PtyCleanup(String);

impl Drop for PtyCleanup {
    fn drop(&mut self) {
        let _ = mosaico::pty::kill(&self.0);
    }
}

fn launch_no_hook(home: &Home, agent: &str, channel: &str, mode: &str) {
    let work_dir = home.dir.path().join(channel);
    add_workspace_mapping(home, channel, &work_dir);
    configure_pty_agent(home, agent, mode);
    let out = run_cli_with_env_in_dir(home, &[agent], &[], &work_dir);
    assert!(
        out.status.success(),
        "launch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn launch_command_resolves_discovered_claude_profile_without_agent_json() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("launch-native-claude");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let profile = home.dir.path().join(".claude/agents/writing-partner.md");
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(
        &profile,
        "---\nname: writing-partner\ndescription: Helps shape written work\n---\nWrite carefully\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(
        home.dir.path().join("bin/opencode"),
        home.dir.path().join("bin/claude"),
    )
    .unwrap();
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        r#"{"claude-pty":{"harness":"claude-code","transport":"pty","args":["forever"]}}"#,
    )
    .unwrap();

    let isolated_home = home.dir.path().to_string_lossy().into_owned();
    let out = run_cli_with_env_in_dir(
        &home,
        &["writing-partner"],
        &[("HOME", &isolated_home)],
        &work_dir,
    );
    assert!(
        out.status.success(),
        "native Claude launch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Launched "),
        "launch did not report its public handle: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!home.dir.path().join("agents/writing-partner.json").exists());

    let meta = mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|meta| meta.agent == "writing-partner")
        .expect("launched writing-partner PTY metadata");
    assert_eq!(
        meta.command,
        ["claude", "forever", "--agent", "writing-partner"]
    );
    let cleanup = PtyCleanup(meta.id);
    drop(cleanup);
    stop_daemon(&home);
}

#[test]
fn launch_lists_and_starts_harness_despite_invalid_same_named_agent() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);
    std::fs::create_dir_all(home.dir.path().join(".config/opencode")).unwrap();
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        r#"{"opencode-pty":{"harness":"opencode","transport":"pty","args":["forever"]}}"#,
    )
    .unwrap();
    mosaico::identity::add_local_agent(
        home.dir.path(),
        "opencode",
        "opencode",
        None,
        mosaico::util::now_secs(),
    )
    .unwrap();
    let channel = unique_session("launch-bare-harness");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let isolated_home = home.dir.path().to_string_lossy().into_owned();
    let xdg = home
        .dir
        .path()
        .join(".config")
        .to_string_lossy()
        .into_owned();

    let listed = run_cli_with_env_in_dir(
        &home,
        &["agents"],
        &[("HOME", &isolated_home), ("XDG_CONFIG_HOME", &xdg)],
        &work_dir,
    );
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("opencode"));

    let launched = run_cli_with_env_in_dir(
        &home,
        &["opencode", "--", "--yolo"],
        &[("HOME", &isolated_home), ("XDG_CONFIG_HOME", &xdg)],
        &work_dir,
    );
    assert!(
        launched.status.success(),
        "bare harness launch failed: {}",
        String::from_utf8_lossy(&launched.stderr)
    );
    let meta = mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|meta| meta.agent == "opencode")
        .expect("launched bare opencode harness metadata");
    assert_eq!(meta.command, ["opencode", "forever", "--yolo"]);
    let stale = mosaico::identity::agent_launch_config(home.dir.path(), "opencode").unwrap();
    assert_eq!(stale.harness, "opencode");

    let cleanup = PtyCleanup(meta.id);
    drop(cleanup);
    stop_daemon(&home);
}

#[test]
fn launch_command_bootstraps_session_without_child_session_start_hook() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("launch-bootstrap");
    let agent = "launch-no-hook-agent";
    launch_no_hook(&home, agent, &channel, "forever");

    let rec = wait_for_alive(&home, agent, &channel);
    let pty_id = Store::open(&home.store_path())
        .unwrap()
        .locators_for_pubkey(&rec.pubkey)
        .unwrap()
        .into_iter()
        .find(|locator| locator.locator_kind == "pty")
        .map(|locator| locator.locator_value)
        .expect("launch must register its PTY locator before returning");

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}

#[test]
fn pty_agent_receives_the_signer_matching_its_assigned_pubkey() {
    use nostr::Keys;

    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("launch-agent-nsec");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let capture = home.dir.path().join("captured-agent-identity");
    let capture_arg = capture.to_string_lossy().into_owned();
    let agent = "launch-agent-nsec";
    configure_pty_agent_with_args(&home, agent, &["capture-identity", &capture_arg]);
    let isolated_home = home.dir.path().to_string_lossy().into_owned();

    let out = run_cli_with_env_in_dir(&home, &[agent], &[("HOME", &isolated_home)], &work_dir);
    assert!(
        out.status.success(),
        "launch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let metadata = mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|meta| meta.agent == agent)
        .expect("launched PTY metadata");
    let pty_id = metadata.id.clone();
    let cleanup = PtyCleanup(pty_id.clone());
    assert!(
        wait_until(Duration::from_secs(10), || capture.exists()),
        "PTY harness did not capture its assigned identity; command={:?}; stderr={}",
        metadata.command,
        String::from_utf8_lossy(&out.stderr)
    );

    let captured = std::fs::read_to_string(&capture).unwrap();
    let mut lines = captured.lines();
    let pubkey = lines.next().expect("captured pubkey");
    let nsec = lines.next().expect("captured nsec");
    assert_eq!(Keys::parse(nsec).unwrap().public_key().to_hex(), pubkey);
    assert!(Store::open(&home.store_path())
        .unwrap()
        .get_session(pubkey)
        .unwrap()
        .is_some());

    drop(cleanup);
    assert!(
        wait_until(Duration::from_secs(5), || !mosaico::pty::is_live(&pty_id)),
        "identity test PTY was not reaped"
    );
    stop_daemon(&home);
}

#[test]
fn supervisor_exit_retires_the_bootstrapped_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("launch-exit");
    let agent = "launch-exit-agent";
    launch_no_hook(&home, agent, &channel, "sleep-2");
    let pty_id = mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|metadata| metadata.agent == agent)
        .map(|metadata| metadata.id)
        .expect("PTY metadata");
    let _attached = mosaico::pty::attach_stream(&pty_id, 24, 80).expect("attach before exit");
    let rec = wait_for_alive(&home, agent, &channel);

    assert!(
        wait_until(Duration::from_secs(10), || {
            Store::open(&home.store_path())
                .and_then(|store| store.get_session(&rec.pubkey))
                .ok()
                .flatten()
                .is_some_and(|session| !session.is_running())
        }),
        "supervisor exit did not retire session {}; daemon_log={}",
        rec.pubkey,
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    let stopped = Store::open(&home.store_path())
        .unwrap()
        .get_session(&rec.pubkey)
        .unwrap()
        .unwrap();
    assert_eq!(
        stopped.stop_reason,
        Some(mosaico::state::StopReason::AttachedCleanExit)
    );
    let standing = Store::open(&home.store_path())
        .unwrap()
        .get_session_standing(&rec.pubkey, &channel)
        .unwrap()
        .unwrap();
    assert_eq!(standing.retain_until, stopped.stopped_at);
    assert!(
        wait_until(Duration::from_secs(25), || {
            refresh_channel_members(&channel);
            Store::open(&home.store_path())
                .map(|store| {
                    !store
                        .is_channel_member(&channel, &rec.pubkey)
                        .unwrap_or(true)
                })
                .unwrap_or(false)
        }),
        "clean headed exit did not remove relay standing"
    );
    stop_daemon(&home);
}

#[test]
fn headless_clean_exit_retains_standing_for_one_hour() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("launch-headless-exit");
    let agent = "launch-headless-exit-agent";
    launch_no_hook(&home, agent, &channel, "sleep-2");
    let rec = wait_for_alive(&home, agent, &channel);
    assert!(wait_until(Duration::from_secs(10), || {
        Store::open(&home.store_path())
            .and_then(|store| store.get_session(&rec.pubkey))
            .ok()
            .flatten()
            .is_some_and(|session| !session.is_running())
    }));
    let store = Store::open(&home.store_path()).unwrap();
    let stopped = store.get_session(&rec.pubkey).unwrap().unwrap();
    assert_eq!(
        stopped.stop_reason,
        Some(mosaico::state::StopReason::HeadlessExit)
    );
    let standing = store
        .get_session_standing(&rec.pubkey, &channel)
        .unwrap()
        .unwrap();
    assert_eq!(
        standing.retain_until,
        stopped.stopped_at + mosaico::state::STOPPED_STANDING_RETENTION_SECS
    );
    stop_daemon(&home);
}

#[test]
fn unreachable_supervisor_fallback_confirms_the_harness_is_dead() {
    use std::os::unix::fs::PermissionsExt as _;

    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("unreachable-supervisor");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "ignore-hup-agent";
    configure_pty_agent(&home, agent, "forever");
    let shim = home.dir.path().join("bin/opencode");
    std::fs::write(&shim, "#!/bin/sh\ntrap '' HUP\nwhile :; do sleep 1; done\n").unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

    let pty_id = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.unwrap();
        client
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": channel,
                    "channel": channel,
                    "cwd": work_dir,
                }),
            )
            .await
            .unwrap()["pty_id"]
            .as_str()
            .unwrap()
            .to_string()
    });
    let session = wait_for_alive(&home, agent, &channel);
    let mut child_pid = None;
    assert!(wait_until(Duration::from_secs(5), || {
        child_pid = mosaico::pty::read_all_metadata()
            .into_iter()
            .find(|metadata| metadata.id == pty_id)
            .and_then(|metadata| metadata.child_pid);
        child_pid.is_some()
    }));
    let metadata = pty_meta(&pty_id);
    std::fs::remove_file(&metadata.socket).unwrap();

    assert!(
        wait_until(Duration::from_secs(25), || {
            Store::open(&home.store_path())
                .and_then(|store| store.get_session(&session.pubkey))
                .ok()
                .flatten()
                .is_some_and(|session| !session.is_running())
        }),
        "unreachable supervisor was not retired; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    let child_pid = i32::try_from(child_pid.unwrap()).unwrap();
    assert!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(child_pid), None).is_err(),
        "harness child survived supervisor fallback termination"
    );
    stop_daemon(&home);
}
