use super::{pid_alive, revive_decision, session_still_live, DaemonState};

#[test]
fn nonpositive_pid_is_never_alive() {
    // Defect #3: a synth ACP pid of 0 (`kill(0)` hits the caller's own group)
    // and negative pids (`kill(-n)` hits a whole group) must read as NOT live,
    // so a dead ACP session is never treated as an immortal ghost.
    assert!(!pid_alive(0));
    assert!(!pid_alive(-1));
}

#[test]
fn native_process_requires_a_live_pid() {
    assert!(!revive_decision(false, false, None));
}

#[test]
fn native_process_revives_on_pid_alone() {
    assert!(revive_decision(true, false, None));
}

#[test]
fn admitted_hosted_session_without_a_locator_never_uses_the_pid() {
    assert!(!revive_decision(true, true, None));
}

#[test]
fn live_hosted_endpoint_is_authoritative_without_a_pid() {
    assert!(revive_decision(false, true, Some(true)));
    assert!(revive_decision(true, true, Some(true)));
}

#[test]
fn dead_hosted_endpoint_is_not_revived_despite_a_live_pid() {
    // Guards against PID recycling: the process at `child_pid` is alive but
    // its supervisor socket is gone, so it is not our session.
    assert!(!revive_decision(true, true, Some(false)));
}

#[tokio::test]
async fn missing_hosted_locators_never_fall_back_to_a_live_pid() {
    let state = DaemonState::new_for_test().await;
    for transport in ["pty", "acp", "app-server"] {
        let pubkey = format!("pk-missing-{transport}");
        state
            .with_store(|store| {
                store.reserve_session_with_facts(
                    &crate::state::RegisterSession {
                        pubkey: pubkey.clone(),
                        observed_harness: "codex".into(),
                        agent_slug: "codex".into(),
                        channel_h: "root".into(),
                        child_pid: Some(std::process::id() as i32),
                        transcript_path: None,
                        now: 1,
                    },
                    &crate::state::AdmittedRuntimeFacts {
                        observed_harness: "codex".into(),
                        claimed_harness: String::new(),
                        bundle: format!("codex-{transport}"),
                        transport: transport.into(),
                        endpoint_provenance: "launch".into(),
                    },
                )
            })
            .unwrap();
        let session = state
            .with_store(|store| store.get_session(&pubkey))
            .unwrap()
            .unwrap();
        assert!(!session_still_live(&state, &session));
    }
}
