use super::*;

fn register(
    state: &Arc<DaemonState>,
    pubkey: &str,
    admitted_transport: &str,
    child_pid: i32,
) -> Session {
    state
        .with_store(|store| {
            let registration = crate::state::RegisterSession {
                pubkey: pubkey.into(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                channel_h: "root".into(),
                child_pid: Some(child_pid),
                transcript_path: None,
                now: 1,
            };
            if admitted_transport.is_empty() {
                store.reserve_hook_session_for_test(&registration)?;
            } else {
                store.reserve_session_with_facts(
                    &registration,
                    &crate::state::AdmittedRuntimeFacts {
                        observed_harness: "codex".into(),
                        claimed_harness: String::new(),
                        bundle: format!("codex-{admitted_transport}"),
                        transport: admitted_transport.into(),
                        endpoint_provenance: "launch".into(),
                    },
                )?;
            }
            store
                .get_session(pubkey)?
                .ok_or_else(|| anyhow::anyhow!("registered session disappeared"))
        })
        .unwrap()
}

#[tokio::test]
async fn automatic_unreachable_pty_fails_closed_and_clears_idle() {
    let state = DaemonState::new_for_test().await;
    let rec = register(&state, "pk-unreachable", "pty", std::process::id() as i32);
    state.with_store(|store| {
        store
            .put_session_locator(
                "codex",
                crate::state::LOCATOR_PTY,
                "missing-pty",
                &rec.pubkey,
                1,
            )
            .unwrap();
        store
            .apply_session_presentation_edge(
                &rec.pubkey,
                rec.runtime_generation,
                1,
                crate::state::PresentationState::Headless,
                2,
            )
            .unwrap();
    });
    let headless = state
        .with_store(|store| store.get_session(&rec.pubkey))
        .unwrap()
        .unwrap();

    assert!(terminate_automatic_if_unattached(&state, &headless)
        .await
        .is_err());
    let retained = state
        .with_store(|store| store.get_session(&rec.pubkey))
        .unwrap()
        .unwrap();
    assert_eq!(retained.runtime_state, RuntimeState::Running);
    assert_eq!(
        retained.presentation_state,
        crate::state::PresentationState::Unavailable
    );
    assert_eq!(retained.idle_deadline, 0);
}

#[tokio::test]
async fn idle_eviction_socket_failure_cancels_stopping_and_fails_closed() {
    let state = DaemonState::new_for_test().await;
    let rec = register(
        &state,
        "pk-idle-unreachable",
        "pty",
        std::process::id() as i32,
    );
    let stopping = state.with_store(|store| {
        store
            .put_session_locator(
                "codex",
                crate::state::LOCATOR_PTY,
                "missing-idle-socket",
                &rec.pubkey,
                1,
            )
            .unwrap();
        store
            .apply_session_presentation_edge(
                &rec.pubkey,
                rec.runtime_generation,
                1,
                crate::state::PresentationState::Headless,
                2,
            )
            .unwrap();
        let due_at = 2 + crate::state::HEADLESS_IDLE_TIMEOUT_SECS;
        store
            .reserve_due_idle_eviction(
                &rec.pubkey,
                rec.runtime_generation,
                rec.lifecycle_epoch,
                1,
                due_at,
            )
            .unwrap()
            .unwrap()
    });
    assert_eq!(stopping.runtime_state, RuntimeState::Stopping);

    assert!(terminate_automatic_if_unattached(&state, &stopping)
        .await
        .is_err());
    let retained = state
        .with_store(|store| store.get_session(&rec.pubkey))
        .unwrap()
        .unwrap();
    assert_eq!(retained.runtime_state, RuntimeState::Running);
    assert_eq!(
        retained.presentation_state,
        crate::state::PresentationState::Unavailable
    );
    assert_eq!(retained.idle_deadline, 0);
}

#[tokio::test]
async fn automatic_hosted_termination_without_locator_always_fails_closed() {
    let state = DaemonState::new_for_test().await;
    for transport in ["pty", "acp", "app-server"] {
        let rec = register(
            &state,
            &format!("pk-automatic-missing-{transport}"),
            transport,
            -1,
        );
        assert!(
            terminate_automatic_if_unattached(&state, &rec)
                .await
                .is_err(),
            "{transport} without a locator must remain tracked"
        );
        let retained = state
            .with_store(|store| store.get_session(&rec.pubkey))
            .unwrap()
            .unwrap();
        assert_eq!(retained.runtime_state, RuntimeState::Running);
        assert_eq!(
            retained.presentation_state,
            crate::state::PresentationState::Unavailable
        );
    }
}

#[tokio::test]
async fn admitted_hosted_session_without_locator_refuses_explicit_pid_fallback() {
    let state = DaemonState::new_for_test().await;
    for transport in ["pty", "acp", "app-server"] {
        let rec = register(
            &state,
            &format!("pk-missing-{transport}"),
            transport,
            std::process::id() as i32,
        );
        let error = terminate_explicit(&state, &rec).await.unwrap_err();
        assert!(
            error.to_string().contains("refusing PID fallback"),
            "{error:#}"
        );
    }
}

#[tokio::test]
async fn explicit_native_process_keeps_pid_fallback() {
    let state = DaemonState::new_for_test().await;
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let child_pid = child.id();
    let rec = register(&state, "pk-native-process", "", child_pid as i32);
    let waiter = tokio::task::spawn_blocking(move || child.wait());

    let result = terminate_explicit(&state, &rec).await;
    if result.is_err() {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child_pid as i32),
            Some(nix::sys::signal::Signal::SIGKILL),
        );
    }
    let status = waiter.await.unwrap().unwrap();
    assert_eq!(result.unwrap(), format!("pid={child_pid}"));
    assert!(!status.success());
}
