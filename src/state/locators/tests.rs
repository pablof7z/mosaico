use super::*;

fn registration(pubkey: &str, at: u64) -> RegisterSession {
    RegisterSession {
        pubkey: pubkey.into(),
        observed_harness: "codex".into(),
        agent_slug: "codex".into(),
        channel_h: "root".into(),
        child_pid: None,
        transcript_path: None,
        now: at,
    }
}

#[test]
fn native_resume_is_stored_once_per_pubkey() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&registration("pk", 1))
        .unwrap();
    store
        .put_session_locator("codex", LOCATOR_NATIVE_RESUME, "old", "pk", 2)
        .unwrap();
    store
        .put_session_locator("codex", LOCATOR_NATIVE_RESUME, "new", "pk", 3)
        .unwrap();

    let locator = store.native_resume_locator("pk", "codex").unwrap().unwrap();
    assert_eq!(locator.locator_value, "new");
    assert!(store
        .native_resume_locator("pk", "claude-code")
        .unwrap()
        .is_none());
    assert!(store
        .resolve_pubkey_by_locator("codex", LOCATOR_NATIVE_RESUME, "old")
        .unwrap()
        .is_none());
}

#[test]
fn locator_vocabulary_is_closed() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&registration("pk", 1))
        .unwrap();
    let error = store
        .put_session_locator("codex", "harness_session", "old", "pk", 2)
        .unwrap_err();
    assert!(error.to_string().contains("unknown session locator kind"));
}

#[test]
fn runtime_endpoint_replacement_fences_stale_generation_callbacks() {
    let store = Store::open_memory().unwrap();
    let first = store
        .reserve_hook_session_for_test(&registration("pk", 1))
        .unwrap();
    store
        .put_session_locator("codex", LOCATOR_PTY, "pty-old", "pk", 2)
        .unwrap();
    store
        .mark_runtime_stopped_if_generation("pk", first, StopReason::Crash, 3)
        .unwrap();
    let second = store
        .reserve_hook_session_for_test(&registration("pk", 4))
        .unwrap();
    store
        .put_session_locator("codex", LOCATOR_PTY, "pty-new", "pk", 5)
        .unwrap();
    assert_eq!(
        store.get_session("pk").unwrap().unwrap().state_changed_at,
        5
    );
    store
        .put_session_locator("codex", LOCATOR_PTY, "pty-newer", "pk", 6)
        .unwrap();
    assert_eq!(
        store.get_session("pk").unwrap().unwrap().state_changed_at,
        5,
        "replacing a live delivery endpoint is not a semantic state edge"
    );

    assert!(store
        .session_for_runtime_locator(LOCATOR_PTY, "pty-old")
        .unwrap()
        .is_none());
    assert!(!store
        .clear_runtime_locator_if_generation("pk", LOCATOR_PTY, first)
        .unwrap());
    assert_eq!(
        store
            .session_for_runtime_locator(LOCATOR_PTY, "pty-newer")
            .unwrap()
            .unwrap()
            .runtime_generation,
        second
    );
}

#[test]
fn session_locator_lookup_requires_the_observed_harness_dimension() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&registration("pk", 1))
        .unwrap();
    store
        .put_session_locator("claude-code", LOCATOR_PTY, "foreign", "pk", 3)
        .unwrap();
    store
        .put_session_locator("codex", LOCATOR_PTY, "owned", "pk", 2)
        .unwrap();

    assert_eq!(
        store
            .locator_for_session("pk", "codex", LOCATOR_PTY)
            .unwrap()
            .unwrap()
            .locator_value,
        "owned"
    );
    assert_eq!(
        store
            .locator_for_session("pk", "claude-code", LOCATOR_PTY)
            .unwrap()
            .unwrap()
            .locator_value,
        "foreign"
    );
}
