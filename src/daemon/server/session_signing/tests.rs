use super::*;

#[tokio::test]
async fn reconstructs_signer_from_pubkey_bound_material() {
    let state = DaemonState::new_for_test().await;
    let agent = crate::identity::AgentIdentity {
        slug: "codex".into(),
        keys: Keys::generate(),
        commands: Vec::new(),
        per_session_key: true,
        harness: None,
    };
    let session_id = "opaque-runtime-row";
    let minted = mint_session_identity(
        &state,
        session_id,
        &agent,
        "root",
        SessionIdentityInput::new("native-resume", None),
        None,
    )
    .unwrap();
    let pubkey = minted.identity.pubkey.clone();
    state.with_store(|store| {
        store
            .upsert_session_row(
                session_id,
                &crate::state::RegisterSession {
                    harness: "codex".into(),
                    external_id_kind: "harness_session".into(),
                    external_id: "native-resume".into(),
                    agent_pubkey: pubkey.clone(),
                    agent_slug: "codex".into(),
                    channel_h: "root".into(),
                    child_pid: None,
                    transcript_path: None,
                    resume_id: "native-resume".into(),
                    now: 1,
                },
            )
            .unwrap();
    });

    let reconstructed = state.session_signing_keys(&pubkey).unwrap();

    assert_eq!(reconstructed.public_key().to_hex(), pubkey);
    assert!(state
        .with_store(|store| store.session_signer_salt(&pubkey))
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn reasserted_runtime_reuses_the_pubkey_bound_signer_salt() {
    let state = DaemonState::new_for_test().await;
    let agent = crate::identity::AgentIdentity {
        slug: "codex".into(),
        keys: Keys::generate(),
        commands: Vec::new(),
        per_session_key: true,
        harness: None,
    };
    let first = mint_session_identity(
        &state,
        "runtime-row",
        &agent,
        "root",
        SessionIdentityInput::new("native-resume", None),
        None,
    )
    .unwrap();
    state.with_store(|store| {
        store
            .upsert_session_row(
                "runtime-row",
                &crate::state::RegisterSession {
                    harness: "codex".into(),
                    external_id_kind: "harness_session".into(),
                    external_id: "native-resume".into(),
                    agent_pubkey: first.identity.pubkey.clone(),
                    agent_slug: "codex".into(),
                    channel_h: "root".into(),
                    child_pid: None,
                    transcript_path: None,
                    resume_id: "native-resume".into(),
                    now: 1,
                },
            )
            .unwrap();
        store
            .put_alias(
                "codex",
                "harness_session",
                "native-resume",
                "runtime-row",
                1,
            )
            .unwrap();
    });

    let resumed = mint_session_identity(
        &state,
        "runtime-row",
        &agent,
        "root",
        SessionIdentityInput::new("native-resume", None),
        None,
    )
    .unwrap();

    assert_eq!(resumed.identity.pubkey, first.identity.pubkey);
    assert_eq!(
        resumed.keys.secret_key().to_secret_hex(),
        first.keys.secret_key().to_secret_hex()
    );
}

#[tokio::test]
async fn concurrent_first_registration_converges_on_one_pubkey() {
    let state = DaemonState::new_for_test().await;
    let agent = crate::identity::AgentIdentity {
        slug: "codex".into(),
        keys: Keys::generate(),
        commands: Vec::new(),
        per_session_key: true,
        harness: None,
    };
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let launch = |state: Arc<DaemonState>,
                  agent: crate::identity::AgentIdentity,
                  barrier: Arc<std::sync::Barrier>| {
        tokio::task::spawn_blocking(move || {
            barrier.wait();
            mint_session_identity(
                &state,
                "same-runtime-row",
                &agent,
                "root",
                SessionIdentityInput::new("native-resume", None),
                None,
            )
            .unwrap()
        })
    };
    let first = launch(state.clone(), agent.clone(), barrier.clone());
    let second = launch(state, agent, barrier);
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    let second = second.unwrap();

    assert_eq!(first.identity.pubkey, second.identity.pubkey);
    assert_eq!(
        first.keys.secret_key().to_secret_hex(),
        second.keys.secret_key().to_secret_hex()
    );
}
