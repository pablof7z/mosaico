use super::*;
use nostr_sdk::prelude::Keys;

fn agent() -> crate::identity::AgentIdentity {
    crate::identity::AgentIdentity {
        slug: "codex".into(),
        keys: None,
        per_session_key: true,
        harness: "codex".into(),
        profile: None,
    }
}

#[tokio::test]
async fn fresh_and_resumed_reservations_expose_the_same_assigned_signer() {
    let state = DaemonState::new_for_test().await;
    let agent = agent();
    let fresh = reserve_fresh(
        &state,
        &agent,
        "codex",
        "codex-pty",
        "pty",
        "root",
        None,
        None,
    )
    .unwrap();
    state
        .with_store(|store| store.set_native_resume_locator(&fresh.pubkey, "codex", "native-1", 1))
        .unwrap();
    release(&state, &fresh);

    let resumed = reserve_resume_exact(
        &state,
        &agent,
        &fresh.pubkey,
        "codex",
        "codex",
        "codex-pty",
        "pty",
        "root",
        "root",
    )
    .unwrap();

    assert_eq!(resumed.pubkey, fresh.pubkey);
    assert_eq!(resumed.agent_nsec, fresh.agent_nsec);
    assert_eq!(
        Keys::parse(&resumed.agent_nsec)
            .unwrap()
            .public_key()
            .to_hex(),
        resumed.pubkey
    );
}

#[tokio::test]
async fn exact_resume_keeps_the_persisted_agent_slug() {
    let state = DaemonState::new_for_test().await;
    let developer = crate::identity::AgentIdentity::per_session("developer", "claude-pty");
    let fresh = reserve_fresh(
        &state,
        &developer,
        "claude-code",
        "claude-pty",
        "pty",
        "mosaico",
        Some("mosaico"),
        None,
    )
    .unwrap();
    release(&state, &fresh);

    let resumed = reserve_resume_exact(
        &state,
        &developer,
        &fresh.pubkey,
        "developer",
        "claude-code",
        "claude-pty",
        "pty",
        "mosaico",
        "mosaico",
    )
    .unwrap();
    let session = state
        .with_store(|store| store.get_session(&resumed.pubkey))
        .unwrap()
        .unwrap();

    assert_eq!(session.agent_slug, "developer");
    assert_eq!(resumed.pubkey, fresh.pubkey);
    assert_eq!(resumed.agent_nsec, fresh.agent_nsec);
}

#[tokio::test]
async fn exact_fresh_launch_requires_a_matching_durable_pubkey() {
    let state = DaemonState::new_for_test().await;
    let per_session = agent();
    let error = match reserve_fresh_for_pubkey(
        &state,
        &per_session,
        "codex",
        "codex-pty",
        "pty",
        "root",
        Some("root"),
        "addressed",
    ) {
        Ok(_) => panic!("per-session identity unexpectedly fresh-launched for an old pubkey"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("unknown per-session pubkey"));
    assert!(state
        .with_store(|store| store.get_session("addressed"))
        .unwrap()
        .is_none());

    let keys = Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let durable = crate::identity::AgentIdentity {
        slug: "integrator".into(),
        keys: Some(keys),
        per_session_key: false,
        harness: "codex".into(),
        profile: None,
    };
    let reservation = reserve_fresh_for_pubkey(
        &state,
        &durable,
        "codex",
        "codex-pty",
        "pty",
        "root",
        Some("root"),
        &pubkey,
    )
    .unwrap();
    assert_eq!(reservation.pubkey, pubkey);
    release(&state, &reservation);
}

#[tokio::test]
async fn stopped_zero_turn_session_can_fresh_relaunch_with_exact_signer() {
    let state = DaemonState::new_for_test().await;
    let agent = agent();
    let first = reserve_fresh(
        &state,
        &agent,
        "codex",
        "codex-pty",
        "pty",
        "root",
        Some("root"),
        None,
    )
    .unwrap();
    release(&state, &first);

    let relaunched = reserve_fresh_for_pubkey(
        &state,
        &agent,
        "codex",
        "codex-pty",
        "pty",
        "root",
        Some("root"),
        &first.pubkey,
    )
    .unwrap();

    assert_eq!(relaunched.pubkey, first.pubkey);
    assert_eq!(relaunched.agent_nsec, first.agent_nsec);
    assert!(relaunched.runtime_generation > first.runtime_generation);
    release(&state, &relaunched);
}
