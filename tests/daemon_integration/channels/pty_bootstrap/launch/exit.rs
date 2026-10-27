//! How a launched runtime's exit is classified, and that exit never leaves
//! the channel.
use super::*;

/// Launch `agent` the way a person does: from a terminal, which selects the
/// interactive PTY transport. The CLI is detached once it hands off.
fn launch_in_terminal(home: &Home, agent: &str, channel: &str, mode: &str) -> TerminalRun {
    let work_dir = home.dir.path().join(channel);
    add_workspace_mapping(home, channel, &work_dir);
    configure_pty_agent(home, agent, mode);
    let run = run_cli_in_terminal(home, &[agent], &[], &work_dir);
    assert!(run.handed_off, "launch failed: {}", run.output);
    run
}

fn pty_id_for_agent(agent: &str) -> String {
    mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|metadata| metadata.agent == agent)
        .map(|metadata| metadata.id)
        .expect("PTY metadata")
}

fn wait_for_stopped(home: &Home, pubkey: &str) -> mosaico::state::Session {
    assert!(
        wait_until(Duration::from_secs(10), || {
            Store::open(&home.store_path())
                .and_then(|store| store.get_session(pubkey))
                .ok()
                .flatten()
                .is_some_and(|session| !session.is_running())
        }),
        "supervisor exit did not retire session {pubkey}; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    Store::open(&home.store_path())
        .unwrap()
        .get_session(pubkey)
        .unwrap()
        .unwrap()
}

fn wait_for_member_standing(home: &Home, pubkey: &str, channel: &str) {
    assert!(
        wait_until(Duration::from_secs(25), || Store::open(&home.store_path())
            .and_then(|store| store.get_session_standing(pubkey, channel))
            .ok()
            .flatten()
            .is_some_and(|standing| {
                standing.state == mosaico::state::StandingState::Member
            })),
        "confirmed membership did not survive runtime stop"
    );
}

#[test]
fn supervisor_exit_stops_runtime_without_leaving_the_channel() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("launch-exit");
    let agent = "launch-exit-agent";
    launch_in_terminal(&home, agent, &channel, "wait-file");
    let pty_id = pty_id_for_agent(agent);
    let _attached = mosaico::pty::attach_stream(&pty_id, 24, 80).expect("attach before exit");
    let rec = wait_for_alive(&home, agent, &channel);

    std::fs::write(home.dir.path().join("release-harness"), "").unwrap();
    let stopped = wait_for_stopped(&home, &rec.pubkey);
    assert_eq!(
        stopped.stop_reason,
        Some(mosaico::state::StopReason::AttachedCleanExit)
    );
    wait_for_member_standing(&home, &rec.pubkey, &channel);
    assert!(Store::open(&home.store_path())
        .unwrap()
        .has_session_route(&rec.pubkey, &channel)
        .unwrap());
    stop_daemon(&home);
}

#[test]
fn headless_clean_exit_preserves_member_standing() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("launch-headless-exit");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "launch-headless-exit-agent";
    configure_pty_agent(&home, agent, "wait-file");
    // A PTY no client ever attached to is headless from its first moment.
    rt().block_on(async {
        Client::connect_or_spawn()
            .await
            .expect("connect")
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": &channel,
                    "channel": &channel,
                    "cwd": &work_dir,
                }),
            )
            .await
            .expect("pty_spawn")
    });
    let rec = wait_for_alive(&home, agent, &channel);

    std::fs::write(home.dir.path().join("release-harness"), "").unwrap();
    let stopped = wait_for_stopped(&home, &rec.pubkey);
    assert_eq!(
        stopped.stop_reason,
        Some(mosaico::state::StopReason::HeadlessExit)
    );
    wait_for_member_standing(&home, &rec.pubkey, &channel);
    stop_daemon(&home);
}
