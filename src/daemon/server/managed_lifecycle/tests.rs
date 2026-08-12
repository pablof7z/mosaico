use super::*;

fn reserve_running(state: &Arc<DaemonState>, pubkey: &str) -> Session {
    state.with_store(|store| {
        store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: pubkey.into(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    launch_channel_h: "room".into(),
                    work_root: "room".into(),
                    child_pid: None,
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
        store.get_session(pubkey).unwrap().unwrap()
    })
}

#[tokio::test]
async fn publish_fence_blocks_forget_and_rejects_the_forgotten_generation() {
    let state = DaemonState::new_for_test().await;
    let session = reserve_running(&state, "publish-fence");
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("room")
                    .metadata("room", "", "", 1)
                    .admins(Vec::new())
                    .members(vec![session.pubkey.clone()])
                    .availability(nmp::nip29::GroupAvailability::SourceUnavailable),
            ]));
            store
                .mark_session_standing_member_if_running(
                    &session.pubkey,
                    "room",
                    session.lifecycle_epoch,
                    1,
                )?
                .context("standing")?;
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();

    let publish_fence = lock_session_route_for_publish(&state, &session, "room")
        .await
        .unwrap();
    let (acquired_tx, mut acquired_rx) = tokio::sync::oneshot::channel();
    let contender = {
        let state = state.clone();
        tokio::spawn(async move {
            let _forget_fence = state.standing_sync.lock().await;
            let _ = acquired_tx.send(());
        })
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut acquired_rx)
            .await
            .is_err(),
        "forget must not cross the final publish boundary"
    );
    drop(publish_fence);
    acquired_rx.await.unwrap();
    contender.await.unwrap();

    state
        .with_store(|store| {
            assert!(store.revoke_session_recovery_if_generation(
                &session.pubkey,
                session.runtime_generation
            )?);
            assert!(store.finalize_session_recovery_revocation(
                &session.pubkey,
                session.runtime_generation,
                2
            )?);
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();

    let error = lock_session_route_for_publish(&state, &session, "room")
        .await
        .expect_err("a forgotten session must never publish with its retained key");
    assert!(
        error
            .to_string()
            .contains("session changed while awaiting channel admission"),
        "{error:#}"
    );
}

#[tokio::test]
async fn admission_revalidation_makes_explicit_absence_win_within_a_lifecycle() {
    let state = DaemonState::new_for_test().await;
    let session = reserve_running(&state, "membership-fence");
    assert!(admission_is_current(
        &state,
        &session.pubkey,
        "room",
        session.runtime_generation,
        session.lifecycle_epoch,
        true,
    ));

    state.with_store(|store| {
        store
            .grant_session_route(&session.pubkey, "room", 1)
            .unwrap();
        store
            .revoke_route_and_mark_absent(&session.pubkey, "room", 2)
            .unwrap();
    });

    assert!(!admission_is_current(
        &state,
        &session.pubkey,
        "room",
        session.runtime_generation,
        session.lifecycle_epoch,
        true,
    ));
    assert!(!admission_is_current(
        &state,
        &session.pubkey,
        "room",
        session.runtime_generation,
        session.lifecycle_epoch,
        false,
    ));
}

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
                    launch_channel_h: "room".into(),
                    work_root: "room".into(),
                    child_pid: Some(42),
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
        command: Vec::new(),
        diagnostic_tail: String::new(),
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
