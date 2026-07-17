use super::*;

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
