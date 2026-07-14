use super::super::*;
use super::reg;

#[test]
fn one_active_runtime_per_pubkey_and_generation_fences_exit() {
    let store = Store::open_memory().unwrap();
    let registration = reg("codex", "pk", "h1");
    let first = store.reserve_session(&registration).unwrap();
    assert!(store.reserve_session(&registration).is_err());

    assert!(store.mark_dead_if_generation("pk", first).unwrap());
    let second = store.reserve_session(&registration).unwrap();
    assert_eq!(second, first + 1);
    assert!(!store.mark_dead_if_generation("pk", first).unwrap());
    assert!(store.get_session("pk").unwrap().unwrap().alive);
}

#[test]
fn typed_locator_resolves_directly_to_pubkey() {
    let store = Store::open_memory().unwrap();
    store.reserve_session(&reg("codex", "pk", "h1")).unwrap();
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
    store.retire_dead_endpoint("pk").unwrap();
    assert!(!store.get_session("pk").unwrap().unwrap().alive);
    assert!(store.locators_for_pubkey("pk").unwrap().is_empty());
}

#[test]
fn explicit_chat_marker_keeps_first_publish() {
    let store = Store::open_memory().unwrap();
    store.reserve_session(&reg("codex", "pk", "h1")).unwrap();
    store
        .mark_session_explicit_chat_published("pk", 1200)
        .unwrap();
    store
        .mark_session_explicit_chat_published("pk", 1300)
        .unwrap();
    assert_eq!(
        store
            .get_session("pk")
            .unwrap()
            .unwrap()
            .explicit_chat_published_at,
        1200
    );
}
