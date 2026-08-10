use super::*;

#[test]
fn unreachable_supervisor_control_does_not_implicitly_terminate_the_harness() {
    use std::os::unix::fs::PermissionsExt as _;

    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("unreachable-supervisor");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "ignore-hup-agent";
    configure_pty_agent(&home, agent, "forever");
    let shim = home.dir.path().join(".local/bin/opencode");
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
    let supervisor_pid = i32::try_from(metadata.supervisor_pid).unwrap();
    std::fs::remove_file(&metadata.socket).unwrap();

    assert!(
        wait_until(Duration::from_secs(25), || std::fs::read_to_string(
            home.dir.path().join("daemon.log")
        )
        .is_ok_and(|log| log.contains("automatic termination denied"))),
        "unreachable supervisor was not retained as unavailable; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    let retained = Store::open(&home.store_path())
        .unwrap()
        .get_session(&session.pubkey)
        .unwrap()
        .unwrap();
    assert!(retained.is_running());
    assert_eq!(
        retained.presentation_state,
        mosaico::state::PresentationState::Unavailable
    );
    let child_pid = i32::try_from(child_pid.unwrap()).unwrap();
    assert!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(child_pid), None).is_ok(),
        "presentation loss implicitly terminated the harness"
    );
    stop_daemon(&home);

    // The missing control socket is the condition under test. Once the
    // assertion is complete, reclaim the exact scenario-owned processes by
    // PID so the test cannot pollute later relay/process checks.
    for pid in [supervisor_pid, child_pid] {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
    if !wait_until(Duration::from_secs(5), || {
        [supervisor_pid, child_pid]
            .into_iter()
            .all(|pid| nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err())
    }) {
        for pid in [supervisor_pid, child_pid] {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
    assert!(
        wait_until(Duration::from_secs(5), || {
            [supervisor_pid, child_pid]
                .into_iter()
                .all(|pid| nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err())
        }),
        "scenario-owned unreachable supervisor or child survived cleanup"
    );
}
