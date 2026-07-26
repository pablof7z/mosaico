use super::*;

fn reserve(store: &Store, at: u64) {
    store
        .reserve_session_with_facts(
            &RegisterSession {
                pubkey: "pk".into(),
                observed_harness: "grok".into(),
                agent_slug: "grok".into(),
                launch_channel_h: "room".into(),
                work_root: "room".into(),
                child_pid: None,
                now: at,
            },
            &AdmittedRuntimeFacts {
                observed_harness: "grok".into(),
                claimed_harness: String::new(),
                bundle: "grok-pty".into(),
                transport: "pty".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
}

#[test]
fn stopped_session_standing_does_not_expire_before_explicit_leave() {
    let store = Store::open_memory().unwrap();
    reserve(&store, 1);
    let running = store.get_session("pk").unwrap().unwrap();
    store
        .mark_session_standing_member_if_running("pk", "room", running.lifecycle_epoch, 2)
        .unwrap()
        .unwrap();
    store
        .mark_runtime_stopped_if_generation("pk", running.runtime_generation, StopReason::Crash, 10)
        .unwrap();
    let standing = store.get_session_standing("pk", "room").unwrap().unwrap();
    assert_eq!(standing.state, StandingState::Member);
    assert!(store.has_session_route("pk", "room").unwrap());
    assert_eq!(store.list_cleanup_due_member_standing().unwrap(), []);
    assert_eq!(
        store.list_stopped_member_standing().unwrap(),
        std::slice::from_ref(&standing)
    );

    store
        .conn
        .execute(
            "DELETE FROM session_channels WHERE pubkey='pk' AND channel_h='room'",
            [],
        )
        .unwrap();
    assert_eq!(
        store.list_cleanup_due_member_standing().unwrap(),
        [standing]
    );
    assert_eq!(store.list_stopped_member_standing().unwrap(), []);
}

#[test]
fn cleanup_due_member_can_be_marked_absent_for_the_same_lifecycle_epoch() {
    let store = Store::open_memory().unwrap();
    store
        .conn
        .execute(
            "INSERT INTO session_standing VALUES ('pk','room','member',1,7,10)",
            [],
        )
        .unwrap();
    let epoch = 1;
    assert!(store
        .mark_member_standing_absent_if_epoch("pk", "room", epoch, 7, 100)
        .unwrap());
    let row = store.get_session_standing("pk", "room").unwrap().unwrap();
    assert_eq!(row.state, StandingState::Absent);
    assert_eq!(row.standing_epoch, epoch + 1);
    assert_eq!(store.list_cleanup_due_member_standing().unwrap(), []);
}
