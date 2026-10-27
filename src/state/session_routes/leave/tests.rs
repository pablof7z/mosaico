use super::*;
use crate::state::session_routes::tests::running;

#[test]
fn a_leaving_route_drops_from_presence_but_stays_due_for_relay_removal() {
    let store = Store::open_memory().unwrap();
    let (generation, session) = running(&store);
    assert_eq!(
        store
            .commit_confirmed_session_admission(
                "pk",
                "room",
                generation,
                session.lifecycle_epoch,
                2
            )
            .unwrap(),
        ConfirmedAdmissionCommit::Committed
    );
    let presence = |store: &Store| {
        let session = store.get_session("pk").unwrap().unwrap();
        crate::session_presence::publication_with_extension(store, &session, false).channels
    };
    assert!(presence(&store).contains("room"));
    let joined = store.list_session_routes("pk").unwrap();

    // Every reconcile re-derives presence from the store, so once the leave
    // begins no reconcile can project the channel again...
    let route = store
        .begin_route_leave("pk", "room")
        .unwrap()
        .expect("route");
    assert!(!presence(&store).contains("room"));
    // ...while the relay removal stays durable work across a restart.
    assert!(store
        .list_cleanup_due_member_standing()
        .unwrap()
        .iter()
        .any(|standing| standing.channel_h == "room"));

    // A failed relay removal restores the exact route.
    store
        .restore_route_after_failed_leave("pk", "room", &route)
        .unwrap();
    assert_eq!(store.list_session_routes("pk").unwrap(), joined);
    assert!(presence(&store).contains("room"));

    // A confirmed removal records absence.
    store
        .begin_route_leave("pk", "room")
        .unwrap()
        .expect("route");
    store.finish_route_leave("pk", "room", 3).unwrap();
    assert!(!store.has_session_route("pk", "room").unwrap());
    assert_eq!(
        store
            .get_session_standing("pk", "room")
            .unwrap()
            .unwrap()
            .state,
        StandingState::Absent
    );
    assert!(store.begin_route_leave("pk", "room").unwrap().is_none());
}
