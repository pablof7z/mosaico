use super::*;

#[tokio::test]
async fn replay_finalizes_reserved_idle_stop_once() {
    let home = tempfile::tempdir().unwrap();
    let _env = crate::test_env::EnvGuard::set("MOSAICO_HOME", home.path());
    let state = DaemonState::new_for_test().await;
    let pty_id = "replay-idle-pty";
    let stopping = state.with_store(|store| {
        store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: "replay-idle".into(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    channel_h: "room".into(),
                    child_pid: Some(42),
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
            .unwrap();
        let running = store.get_session("replay-idle").unwrap().unwrap();
        store
            .put_session_locator(
                "codex",
                crate::state::LOCATOR_PTY,
                pty_id,
                &running.pubkey,
                2,
            )
            .unwrap();
        store
            .apply_session_presentation_edge(
                &running.pubkey,
                running.runtime_generation,
                1,
                PresentationState::Headless,
                10,
            )
            .unwrap();
        store
            .reserve_due_idle_eviction(
                &running.pubkey,
                running.runtime_generation,
                running.lifecycle_epoch,
                1,
                10 + crate::state::HEADLESS_IDLE_TIMEOUT_SECS,
            )
            .unwrap()
            .unwrap()
    });
    let exited_at = stopping.stopped_at + 1;
    crate::pty::persist_exit_report(&crate::pty::SupervisorExitReport {
        pty_id: pty_id.into(),
        child_success: None,
        child_exit_code: None,
        presentation: crate::pty::PresentationSnapshot {
            attached_clients: 0,
            attachment_epoch: 1,
            changed_at: exited_at,
        },
        recorded_at: exited_at,
    })
    .unwrap();

    replay_supervisor_exits(&state).await;

    let stopped = state
        .with_store(|store| store.get_session(&stopping.pubkey))
        .unwrap()
        .unwrap();
    assert_eq!(stopped.runtime_state, RuntimeState::Stopped);
    assert_eq!(stopped.stop_reason, Some(StopReason::IdleEvicted));
    assert_eq!(stopped.stopped_at, exited_at);
    assert!(crate::pty::read_exit_reports().is_empty());

    assert!(!supervisor_exited(
        &state,
        pty_id,
        None,
        crate::pty::PresentationSnapshot::default(),
        exited_at + 1,
    )
    .await
    .unwrap());
    let replayed = state
        .with_store(|store| store.get_session(&stopping.pubkey))
        .unwrap()
        .unwrap();
    assert_eq!(replayed.stop_reason, Some(StopReason::IdleEvicted));
    assert_eq!(replayed.stopped_at, exited_at);
}
