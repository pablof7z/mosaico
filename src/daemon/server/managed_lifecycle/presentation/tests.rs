use super::*;

fn reserve_rpc_session(
    state: &Arc<DaemonState>,
    pubkey: &str,
    harness: &str,
    transport: &str,
    locator_kind: &str,
) {
    state.with_store(|store| {
        store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: pubkey.into(),
                    observed_harness: harness.into(),
                    agent_slug: pubkey.into(),
                    channel_h: "room".into(),
                    child_pid: Some(42),
                    transcript_path: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: harness.into(),
                    claimed_harness: String::new(),
                    bundle: format!("{harness}-{transport}"),
                    transport: transport.into(),
                    endpoint_provenance: "launch".into(),
                },
            )
            .unwrap();
        store
            .put_session_locator(harness, locator_kind, pubkey, pubkey, 1)
            .unwrap();
    });
}

#[tokio::test]
async fn rpc_transports_reconcile_as_headless_and_arm_idle_eviction() {
    let state = DaemonState::new_for_test().await;
    reserve_rpc_session(
        &state,
        "acp-pk",
        "claude-code",
        "acp",
        crate::state::LOCATOR_ACP,
    );
    reserve_rpc_session(
        &state,
        "app-server-pk",
        "codex",
        "app-server",
        crate::state::LOCATOR_APP_SERVER,
    );
    let before = now_secs();

    reconcile(&state).await;

    let after = now_secs();
    state.with_store(|store| {
        for pubkey in ["acp-pk", "app-server-pk"] {
            let session = store.get_session(pubkey).unwrap().unwrap();
            assert_eq!(session.presentation_state, PresentationState::Headless);
            assert!((before..=after).contains(&session.idle_since));
            assert_eq!(
                session.idle_deadline,
                session
                    .idle_since
                    .saturating_add(crate::state::HEADLESS_IDLE_TIMEOUT_SECS)
            );
        }
    });
}

#[tokio::test]
async fn failed_pty_probe_immediately_persists_unavailable_and_clears_idle() {
    let state = DaemonState::new_for_test().await;
    let pubkey = "unreachable-pty";
    state.with_store(|store| {
        store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: pubkey.into(),
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
            .unwrap();
        let session = store.get_session(pubkey).unwrap().unwrap();
        store
            .put_session_locator(
                "codex",
                crate::state::LOCATOR_PTY,
                "missing-presentation-socket",
                pubkey,
                1,
            )
            .unwrap();
        store
            .apply_session_presentation_edge(
                pubkey,
                session.runtime_generation,
                1,
                PresentationState::Headless,
                2,
            )
            .unwrap();
    });

    reconcile(&state).await;

    let retained = state
        .with_store(|store| store.get_session(pubkey))
        .unwrap()
        .unwrap();
    assert_eq!(retained.runtime_state, RuntimeState::Running);
    assert_eq!(retained.presentation_state, PresentationState::Unavailable);
    assert_eq!(retained.idle_deadline, 0);
}
