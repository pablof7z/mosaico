use super::*;

#[test]
fn exact_running_supervisor_is_revived_without_termination() {
    assert_eq!(
        decision_for_observation(Some(crate::pty::OwnedSupervisorState::Running)),
        Decision::ReviveUnavailable
    );
}

#[test]
fn ownership_uncertainty_always_fails_closed() {
    for owned in [None, Some(crate::pty::OwnedSupervisorState::Missing)] {
        assert_eq!(decision_for_observation(owned), Decision::RetainUnavailable);
    }
}

#[test]
fn exact_metadata_with_an_absent_supervisor_is_genuinely_gone() {
    assert_eq!(
        decision_for_observation(Some(crate::pty::OwnedSupervisorState::Gone)),
        Decision::Gone
    );
}

#[tokio::test]
async fn startup_reconcile_without_locator_retains_live_pty_as_unavailable() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|store| {
            store.reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: "startup-unavailable".into(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    channel_h: "room".into(),
                    child_pid: Some(std::process::id() as i32),
                    transcript_path: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    bundle: "codex-pty".into(),
                    transport: "pty".into(),
                    endpoint_provenance: "launch".into(),
                },
            )
        })
        .unwrap();
    let session = state
        .with_store(|store| store.get_session("startup-unavailable"))
        .unwrap()
        .unwrap();

    assert_eq!(reconcile(&state, &session), Decision::RetainUnavailable);
    let retained = state
        .with_store(|store| store.get_session("startup-unavailable"))
        .unwrap()
        .unwrap();
    assert_eq!(retained.runtime_state, crate::state::RuntimeState::Running);
    assert_eq!(
        retained.presentation_state,
        crate::state::PresentationState::Unavailable
    );
    assert_eq!(retained.idle_deadline, 0);
}
