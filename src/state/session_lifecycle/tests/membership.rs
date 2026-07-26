use super::*;

#[test]
fn finalized_stop_preserves_membership_until_explicit_leave() {
    let store = seed();
    let initial = running(&store);
    store
        .mark_session_standing_member_if_running("pk", "room", initial.lifecycle_epoch, 2)
        .unwrap()
        .unwrap();
    store
        .apply_session_presentation_edge(
            "pk",
            initial.runtime_generation,
            1,
            PresentationState::Headless,
            10,
        )
        .unwrap();
    let stopping = store
        .reserve_due_idle_eviction(
            "pk",
            initial.runtime_generation,
            initial.lifecycle_epoch,
            1,
            10 + HEADLESS_IDLE_TIMEOUT_SECS,
        )
        .unwrap()
        .unwrap();
    store
        .finalize_runtime_stopped_if_epoch(
            "pk",
            initial.runtime_generation,
            stopping.lifecycle_epoch,
            StopReason::IdleEvicted,
            stopping.stopped_at,
        )
        .unwrap()
        .unwrap();
    let standing = store.list_session_standing("pk").unwrap();
    assert_eq!(standing[0].state, StandingState::Member);
    assert!(store.has_session_route("pk", "room").unwrap());
    assert!(store.list_cleanup_due_member_standing().unwrap().is_empty());
}
