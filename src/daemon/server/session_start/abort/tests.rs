use super::*;

#[tokio::test]
async fn dropping_guard_after_claim_rolls_back_every_provisional_binding() {
    let state = DaemonState::new_for_test().await;
    let keys = nostr_sdk::prelude::Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let agent = crate::identity::AgentIdentity {
        slug: "chief".into(),
        keys,
        commands: Vec::new(),
        per_session_key: false,
        harness: None,
    };
    let minted = mint_session_identity(&state, "orphan", &agent, "root", "", None).unwrap();
    assert!(minted.durable_claim_acquired);
    state.with_store(|s| {
        s.put_alias("codex", "harness_session", "native", "orphan", 1)
            .unwrap();
    });

    drop(DurableStartGuard::new(&state, "orphan", true));

    assert!(!state.session_keys.lock().unwrap().contains_key("orphan"));
    state.with_store(|s| {
        assert!(s
            .live_durable_session_for_pubkey(&pubkey)
            .unwrap()
            .is_none());
        assert!(s
            .get_identity(&pubkey)
            .unwrap()
            .is_none_or(|identity| !identity.alive));
        assert!(s.aliases_for_session("orphan").unwrap().is_empty());
    });
}
