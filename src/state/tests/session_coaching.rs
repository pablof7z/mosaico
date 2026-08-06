use super::*;

#[test]
fn coaching_claim_is_once_per_runtime_generation_and_code() {
    let store = Store::open_memory().unwrap();
    let registration = reg("codex", "pk", "room");
    let first_generation = store.reserve_hook_session_for_test(&registration).unwrap();

    assert!(store
        .claim_session_coaching("pk", first_generation, "unhosted_return_path", 10)
        .unwrap());
    assert!(!store
        .claim_session_coaching("pk", first_generation, "unhosted_return_path", 11)
        .unwrap());
    assert!(store
        .claim_session_coaching("pk", first_generation, "another_code", 12)
        .unwrap());

    store
        .mark_runtime_stopped_if_generation("pk", first_generation, StopReason::Unknown, 20)
        .unwrap();
    let second_generation = store
        .reserve_hook_session_for_test(&RegisterSession {
            now: 30,
            ..registration
        })
        .unwrap();
    assert!(store
        .claim_session_coaching("pk", second_generation, "unhosted_return_path", 31)
        .unwrap());
}
