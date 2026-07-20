use super::*;

#[tokio::test]
async fn mapped_pubkey_overrides_stale_hook_agent_claim() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|store| {
            store.reserve_hook_session_for_test(&crate::state::RegisterSession {
                pubkey: "mapped-pubkey".into(),
                observed_harness: "claude-code".into(),
                agent_slug: "claude".into(),
                channel_h: "mosaico".into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            })
        })
        .unwrap();
    let mut params = SessionStartParams {
        agent: "developer".into(),
        profile: Some("developer".into()),
        ..Default::default()
    };

    let persisted = reconcile_agent_from_pubkey(&state, &mut params, Some("mapped-pubkey"))
        .unwrap()
        .unwrap();

    assert_eq!(persisted.agent_slug, "claude");
    assert_eq!(params.agent, "claude");
    assert_eq!(params.profile, None);
}

#[tokio::test]
async fn endpoint_without_kind_cannot_resolve_or_persist() {
    let state = DaemonState::new_for_test().await;
    let params = SessionStartParams {
        pty_session: Some("untyped-endpoint".into()),
        ..Default::default()
    };

    let resolve_error = resolve_existing_pubkey(&state, &params, "codex")
        .unwrap_err()
        .to_string();
    assert!(resolve_error.contains("requires explicit endpoint_kind"));

    let store = crate::state::Store::open_memory().unwrap();
    let persist_error = bind_locators(&store, &params, "codex", "pk", 1)
        .unwrap_err()
        .to_string();
    assert!(persist_error.contains("requires explicit endpoint_kind"));
    assert!(store.locators_for_pubkey("pk").unwrap().is_empty());
}

#[test]
fn bind_locators_records_native_resume_locator() {
    let store = crate::state::Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: "pk".into(),
            observed_harness: "claude-code".into(),
            agent_slug: "claude".into(),
            channel_h: "root".into(),
            child_pid: None,
            transcript_path: None,
            now: 1,
        })
        .unwrap();
    let params = SessionStartParams {
        pty_session: Some("acp-claude-endpoint".into()),
        endpoint_kind: Some(crate::session_host::transport::TransportKind::Acp),
        resume_id: Some("acp-session-123".into()),
        ..Default::default()
    };

    bind_locators(&store, &params, "claude-code", "pk", 1).unwrap();

    let locator = store
        .native_resume_locator("pk", "claude-code")
        .unwrap()
        .expect("hosted ACP launch must record a native_resume locator");
    assert_eq!(locator.locator_value, "acp-session-123");
}
