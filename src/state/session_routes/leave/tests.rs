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
    assert!(store
        .restore_route_after_failed_leave("pk", "room", &route)
        .unwrap());
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

fn joined_and_leaving(store: &Store) -> LeavingRoute {
    let (generation, session) = running(store);
    store
        .commit_confirmed_session_admission("pk", "room", generation, session.lifecycle_epoch, 2)
        .unwrap();
    store
        .begin_route_leave("pk", "room")
        .unwrap()
        .expect("route")
}

#[test]
fn a_failed_leave_never_restores_a_route_onto_a_forgotten_session() {
    let store = Store::open_memory().unwrap();
    let route = joined_and_leaving(&store);
    let generation = store.get_session("pk").unwrap().unwrap().runtime_generation;
    store
        .revoke_session_recovery_if_generation("pk", generation)
        .unwrap();
    store
        .finalize_session_recovery_revocation("pk", generation, 3)
        .unwrap();

    assert!(!store
        .restore_route_after_failed_leave("pk", "room", &route)
        .unwrap());
    assert!(!store.has_session_route("pk", "room").unwrap());
}

#[test]
fn a_failed_leave_never_overwrites_a_concurrent_rejoin() {
    let store = Store::open_memory().unwrap();
    let route = joined_and_leaving(&store);
    let session = store.get_session("pk").unwrap().unwrap();
    // A join while the leave was unlocked re-admits the channel.
    store
        .commit_confirmed_session_admission(
            "pk",
            "room",
            session.runtime_generation,
            session.lifecycle_epoch,
            9,
        )
        .unwrap();
    let rejoined = store.list_session_routes("pk").unwrap();

    assert!(!store
        .restore_route_after_failed_leave("pk", "room", &route)
        .unwrap());
    assert_eq!(store.list_session_routes("pk").unwrap(), rejoined);
}

#[test]
fn a_failed_leave_does_not_restore_after_its_standing_changed() {
    let store = Store::open_memory().unwrap();
    let route = joined_and_leaving(&store);
    let session = store.get_session("pk").unwrap().unwrap();
    store
        .mark_session_standing_member_if_running("pk", "room", session.lifecycle_epoch, 4)
        .unwrap();

    assert!(!store
        .restore_route_after_failed_leave("pk", "room", &route)
        .unwrap());
}

#[test]
fn a_failed_leave_does_not_restore_onto_a_session_being_forgotten() {
    let store = Store::open_memory().unwrap();
    let route = joined_and_leaving(&store);
    let generation = store.get_session("pk").unwrap().unwrap().runtime_generation;
    // Forget's first, durable step: recovery is revoked before teardown.
    store
        .revoke_session_recovery_if_generation("pk", generation)
        .unwrap();

    assert!(!store
        .restore_route_after_failed_leave("pk", "room", &route)
        .unwrap());
}
