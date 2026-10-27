use super::super::*;
use super::reg;

#[test]
fn one_active_runtime_per_pubkey_and_generation_fences_exit() {
    let store = Store::open_memory().unwrap();
    let registration = reg("codex", "pk", "h1");
    let first = store.reserve_hook_session_for_test(&registration).unwrap();
    assert!(store.reserve_hook_session_for_test(&registration).is_err());

    assert!(store
        .mark_runtime_stopped_if_generation("pk", first, StopReason::Unknown, 1)
        .unwrap());
    let second = store.reserve_hook_session_for_test(&registration).unwrap();
    assert_eq!(second, first + 1);
    assert!(!store
        .mark_runtime_stopped_if_generation("pk", first, StopReason::Unknown, 2)
        .unwrap());
    assert!(store.get_session("pk").unwrap().unwrap().is_running());
}

#[test]
fn typed_locator_resolves_directly_to_pubkey() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&reg("codex", "pk", "h1"))
        .unwrap();
    store
        .put_session_locator("codex", LOCATOR_PTY, "endpoint", "pk", 2)
        .unwrap();
    assert_eq!(
        store
            .resolve_pubkey_by_locator("codex", LOCATOR_PTY, "endpoint")
            .unwrap()
            .as_deref(),
        Some("pk")
    );
    let generation = store.get_session("pk").unwrap().unwrap().runtime_generation;
    store
        .clear_runtime_locator_if_generation("pk", LOCATOR_PTY, generation)
        .unwrap();
    store
        .mark_runtime_stopped("pk", StopReason::Unknown, 3)
        .unwrap();
    assert!(!store.get_session("pk").unwrap().unwrap().is_running());
    assert!(store.locators_for_pubkey("pk").unwrap().is_empty());
}
