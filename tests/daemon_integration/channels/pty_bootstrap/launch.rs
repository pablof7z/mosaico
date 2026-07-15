use super::*;

fn launch_no_hook(home: &Home, agent: &str, channel: &str, mode: &str) {
    let work_dir = home.dir.path().join(channel);
    add_workspace_mapping(home, channel, &work_dir);
    configure_pty_agent(home, agent, mode);
    let out = run_cli_with_env_in_dir(
        home,
        &["launch", agent, "--workspace", channel],
        &[],
        &work_dir,
    );
    assert!(
        out.status.success(),
        "launch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
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
fn supervisor_exit_retires_the_bootstrapped_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("launch-exit");
    let agent = "launch-exit-agent";
    launch_no_hook(&home, agent, &channel, "sleep-2");
    let rec = wait_for_alive(&home, agent, &channel);

    assert!(
        wait_until(Duration::from_secs(10), || {
            Store::open(&home.store_path())
                .and_then(|store| store.get_session(&rec.pubkey))
                .ok()
                .flatten()
                .is_some_and(|session| !session.alive)
        }),
        "supervisor exit did not retire session {}",
        rec.pubkey
    );
    stop_daemon(&home);
}
