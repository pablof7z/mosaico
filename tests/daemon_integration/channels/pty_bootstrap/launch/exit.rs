//! How a launched runtime's exit is classified, and that exit never leaves
//! the channel.
use super::*;

/// Spawn `agent`'s PTY over `pty_spawn` with no client attached: headless
/// from its first moment. Its `wait-file` child runs until released.
fn spawn_unattached(home: &Home, agent: &str, channel: &str) -> String {
    let work_dir = home.dir.path().join(channel);
    add_workspace_mapping(home, channel, &work_dir);
    configure_pty_agent(home, agent, "wait-file");
    let spawned = rt().block_on(async {
        Client::connect_or_spawn()
            .await
            .expect("connect")
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": channel,
                    "channel": channel,
                    "cwd": &work_dir,
                }),
            )
            .await
            .expect("pty_spawn")
    });
    spawned["pty_id"].as_str().expect("pty_id").to_string()
}

fn release_harness(home: &Home) {
    std::fs::write(home.dir.path().join("release-harness"), "").unwrap();
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
    let pty_id = spawn_unattached(&home, agent, &channel);
    let rec = wait_for_alive(&home, agent, &channel);
    let _attached = mosaico::pty::attach_stream(&pty_id, 24, 80).expect("attach before exit");
    // The supervisor registers a client asynchronously; its presentation edge
    // is the proof that the exit below happens while attached.
    assert!(
        wait_until(Duration::from_secs(10), || Store::open(&home.store_path())
            .and_then(|store| store.get_session(&rec.pubkey))
            .ok()
            .flatten()
            .is_some_and(|session| {
                session.presentation_state == mosaico::state::PresentationState::Headed
            })),
        "the supervisor never reported the attached client"
    );

    release_harness(&home);
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
    let agent = "launch-headless-exit-agent";
    spawn_unattached(&home, agent, &channel);
    let rec = wait_for_alive(&home, agent, &channel);

    release_harness(&home);
    let stopped = wait_for_stopped(&home, &rec.pubkey);
    assert_eq!(
        stopped.stop_reason,
        Some(mosaico::state::StopReason::HeadlessExit)
    );
    wait_for_member_standing(&home, &rec.pubkey, &channel);
    stop_daemon(&home);
}
