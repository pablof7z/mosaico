use super::*;

fn seed() -> Store {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: "pk".into(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            channel_h: "mosaico".into(),
            child_pid: Some(42),
            transcript_path: None,
            now: 1,
        })
        .unwrap();
    store
}

fn session(store: &Store) -> Session {
    store.get_session("pk").unwrap().unwrap()
}

#[test]
fn accumulates_once_on_turn_end_and_runtime_stop() {
    let store = seed();
    let generation = session(&store).runtime_generation;
    assert!(store
        .apply_session_turn_started("pk", generation, 10, None)
        .unwrap());
    assert!(store
        .apply_session_turn_ended("pk", generation, 30)
        .unwrap());
    assert_eq!(session(&store).busy_seconds, 20);

    assert!(!store
        .apply_session_turn_ended("pk", generation, 35)
        .unwrap());
    assert_eq!(session(&store).busy_seconds, 20);

    assert!(store
        .apply_session_turn_started("pk", generation, 40, None)
        .unwrap());
    store
        .mark_runtime_stopped("pk", StopReason::OperatorKill, 55)
        .unwrap();
    assert_eq!(session(&store).busy_seconds, 35);
    assert_eq!(session(&store).turn_started_at, 0);
}
