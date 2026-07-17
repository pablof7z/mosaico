use super::*;

fn ordinary_agent() -> crate::identity::AgentIdentity {
    crate::identity::AgentIdentity {
        slug: "codex".into(),
        keys: None,
        per_session_key: true,
        harness: "codex".into(),
        profile: None,
    }
}

#[tokio::test]
async fn reconstructs_signer_from_pubkey_bound_material() {
    let state = DaemonState::new_for_test().await;
    let prepared = prepare_session_identity(&state, &ordinary_agent(), None).unwrap();
    let pubkey = prepared.identity.pubkey.clone();
    state
        .with_store(|store| {
            store.reserve_hook_session_for_test(&crate::state::RegisterSession {
                pubkey: pubkey.clone(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                channel_h: "root".into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            })
        })
        .unwrap();

    let reconstructed = state.session_signing_keys(&pubkey).unwrap();

    assert_eq!(reconstructed.public_key().to_hex(), pubkey);
    assert_eq!(
        reconstructed.secret_key().to_secret_hex(),
        prepared.keys.secret_key().to_secret_hex()
    );
}

#[tokio::test]
async fn fresh_preparations_get_distinct_pubkeys_and_handles() {
    let state = DaemonState::new_for_test().await;
    let agent = ordinary_agent();

    let first = prepare_session_identity(&state, &agent, None).unwrap();
    let second = prepare_session_identity(&state, &agent, None).unwrap();

    assert_ne!(first.identity.pubkey, second.identity.pubkey);
    assert_ne!(first.identity.handle, second.identity.handle);
}

#[tokio::test]
async fn custom_handle_conflict_rolls_back_prepared_signer() {
    let state = DaemonState::new_for_test().await;
    let agent = ordinary_agent();
    let first = prepare_session_identity(&state, &agent, Some("research")).unwrap();

    let error = match prepare_session_identity(&state, &agent, Some("research")) {
        Ok(_) => panic!("duplicate custom handle should be rejected"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("already in use"));
    assert_eq!(first.identity.handle, "research-codex");
    assert!(state
        .with_store(|store| store.session_signer_salt(&first.identity.pubkey))
        .unwrap()
        .is_some());
}
