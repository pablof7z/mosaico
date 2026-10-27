use super::*;

/// Reserve a fresh derived identity named only by `hint`, as session start does.
fn reserve_with_hint(
    store: &Store,
    pubkey: &str,
    hint: Option<&str>,
    now: u64,
) -> HandleAllocation {
    store
        .reserve_derived_identity("codex", None, hint, now, |_| Ok(((), pubkey.to_string())))
        .unwrap()
        .2
}

#[test]
fn handle_conflict_rolls_back_signer() {
    let store = Store::open_memory().unwrap();
    store
        .allocate_custom_handle("owner", "codex", "named", 1)
        .unwrap();
    let error = store
        .reserve_derived_identity("codex", Some("named"), None, 2, |_| {
            Ok(((), "new-pk".to_string()))
        })
        .unwrap_err();

    assert!(error.to_string().contains("already in use"));
    assert!(store.session_signer_salt("new-pk").unwrap().is_none());
    assert_eq!(
        store.pubkey_for_handle("named-codex").unwrap().as_deref(),
        Some("owner")
    );
}

#[test]
fn hint_normalizes_free_text_into_a_handle() {
    let store = Store::open_memory().unwrap();
    let allocation = reserve_with_hint(&store, "pk", Some("My Cool Agent!"), 10);
    assert_eq!(allocation.handle, "my-cool-agent-codex");
}

#[test]
fn hint_falls_back_to_codename_on_live_collision() {
    let store = Store::open_memory().unwrap();
    store
        .allocate_custom_handle("owner", "codex", "taken", 1)
        .unwrap();

    let allocation = reserve_with_hint(&store, "new", Some("taken"), 2);

    assert_ne!(allocation.handle, "taken-codex");
    assert_eq!(
        store.pubkey_for_handle("taken-codex").unwrap().as_deref(),
        Some("owner")
    );
}

#[test]
fn blank_hint_behaves_like_no_hint() {
    let without_hint = reserve_with_hint(&Store::open_memory().unwrap(), "pk", None, 10);
    let blank_hint = reserve_with_hint(&Store::open_memory().unwrap(), "pk", Some("   "), 10);
    assert_eq!(blank_hint, without_hint);
}

#[test]
fn hint_respects_a_stopped_sessions_grace_window() {
    let store = Store::open_memory().unwrap();
    store
        .allocate_custom_handle("stopped", "codex", "research", 100)
        .unwrap();
    store
        .conn
        .execute("UPDATE handle_leases SET live=0 WHERE pubkey='stopped'", [])
        .unwrap();

    let inside = reserve_with_hint(
        &store,
        "early",
        Some("research"),
        100 + HANDLE_LEASE_GRACE_SECS - 1,
    );
    assert_ne!(inside.handle, "research-codex");
    assert_eq!(
        store
            .pubkey_for_handle("research-codex")
            .unwrap()
            .as_deref(),
        Some("stopped")
    );

    let after = reserve_with_hint(
        &store,
        "late",
        Some("research"),
        100 + HANDLE_LEASE_GRACE_SECS,
    );
    assert_eq!(after.handle, "research-codex");
    assert_eq!(after.reclaimed_pubkey.as_deref(), Some("stopped"));
    assert_eq!(
        store
            .pubkey_for_handle("research-codex")
            .unwrap()
            .as_deref(),
        Some("late")
    );
}
