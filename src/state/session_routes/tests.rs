use super::*;

fn running(store: &Store) -> (u64, Session) {
    let registration = RegisterSession {
        pubkey: "pk".into(),
        observed_harness: "grok".into(),
        agent_slug: "grok".into(),
        launch_channel_h: "root".into(),
        work_root: "root".into(),
        child_pid: None,
        now: 1,
    };
    let generation = store
        .reserve_session_with_facts(
            &registration,
            &AdmittedRuntimeFacts {
                observed_harness: "grok".into(),
                claimed_harness: String::new(),
                bundle: "grok-pty".into(),
                transport: "pty".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    (generation, store.get_session("pk").unwrap().unwrap())
}

#[test]
fn route_affinity_survives_standing_removal() {
    let store = Store::open_memory().unwrap();
    store.grant_session_route("pk", "room", 1).unwrap();
    store
        .conn
        .execute(
            "INSERT INTO session_standing VALUES ('pk','room','member',1,2,2)",
            [],
        )
        .unwrap();
    store
        .mark_member_standing_absent_if_epoch("pk", "room", 1, 2, 10)
        .unwrap();

    assert!(store.has_session_route("pk", "room").unwrap());
}

#[test]
fn confirmed_runtime_join_remains_a_member_on_stop() {
    let store = Store::open_memory().unwrap();
    let (generation, session) = running(&store);
    assert_eq!(
        store
            .commit_confirmed_session_admission(
                "pk",
                "joined",
                generation,
                session.lifecycle_epoch,
                2,
            )
            .unwrap(),
        ConfirmedAdmissionCommit::Committed
    );
    store
        .mark_runtime_stopped_if_generation("pk", generation, StopReason::Crash, 10)
        .unwrap();

    let joined = store.get_session_standing("pk", "joined").unwrap().unwrap();
    assert_eq!(joined.state, StandingState::Member);
}

#[test]
fn confirmed_admission_after_runtime_stop_preserves_membership() {
    let store = Store::open_memory().unwrap();
    let (generation, session) = running(&store);
    store
        .mark_runtime_stopped_if_generation("pk", generation, StopReason::Crash, 10)
        .unwrap();

    assert_eq!(
        store
            .commit_confirmed_session_admission(
                "pk",
                "joined",
                generation,
                session.lifecycle_epoch,
                11,
            )
            .unwrap(),
        ConfirmedAdmissionCommit::Committed
    );
    assert!(store.has_session_route("pk", "joined").unwrap());
    assert_eq!(
        store
            .get_session_standing("pk", "joined")
            .unwrap()
            .unwrap()
            .state,
        StandingState::Member
    );
    assert!(store.list_cleanup_due_member_standing().unwrap().is_empty());
}

#[test]
fn compensation_fallback_recognizes_a_committed_admission() {
    let store = Store::open_memory().unwrap();
    let (generation, session) = running(&store);
    store
        .commit_confirmed_session_admission("pk", "joined", generation, session.lifecycle_epoch, 2)
        .unwrap();

    assert_eq!(
        store
            .schedule_confirmed_admission_cleanup(
                "pk",
                "joined",
                generation,
                session.lifecycle_epoch,
                3,
            )
            .unwrap(),
        ConfirmedAdmissionCommit::Committed
    );
}

#[test]
fn compensation_fallback_persists_due_after_primary_write_error() {
    let store = Store::open_memory().unwrap();
    let (generation, session) = running(&store);
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER reject_join_route BEFORE INSERT ON session_channels
             WHEN NEW.channel_h='joined'
             BEGIN SELECT RAISE(FAIL, 'forced route failure'); END;",
        )
        .unwrap();
    assert!(store
        .commit_confirmed_session_admission("pk", "joined", generation, session.lifecycle_epoch, 2,)
        .is_err());

    let result = store
        .schedule_confirmed_admission_cleanup(
            "pk",
            "joined",
            generation,
            session.lifecycle_epoch,
            3,
        )
        .unwrap();
    let ConfirmedAdmissionCommit::CleanupDue(due) = result else {
        panic!("expected durable cleanup")
    };
    assert_eq!(due.state, StandingState::Member);
    assert_eq!(store.list_cleanup_due_member_standing().unwrap(), [due]);
}

#[test]
fn confirmed_admission_preserves_the_original_membership_cutoff() {
    let store = Store::open_memory().unwrap();
    let (generation, session) = running(&store);
    store.grant_session_route("pk", "joined", 2).unwrap();
    let original = route_fence(&store, "pk", "joined");
    insert_chat(&store, "arrived-after-join", "joined", 3);

    store
        .commit_confirmed_session_admission("pk", "joined", generation, session.lifecycle_epoch, 4)
        .unwrap();

    assert_eq!(route_fence(&store, "pk", "joined"), original);
}

#[test]
fn explicit_leave_and_rejoin_resets_both_membership_fences() {
    let store = Store::open_memory().unwrap();
    let (generation, session) = running(&store);
    store.grant_session_route("pk", "joined", 2).unwrap();
    insert_chat(&store, "before-rejoin", "joined", 10);
    let before = route_fence(&store, "pk", "joined");

    assert!(store
        .revoke_route_and_mark_absent("pk", "joined", 11)
        .unwrap());
    store
        .commit_confirmed_session_admission("pk", "joined", generation, session.lifecycle_epoch, 12)
        .unwrap();
    let after = route_fence(&store, "pk", "joined");

    assert_eq!(after.0, 12);
    assert!(after.1 > before.1);
    assert!(!store
        .session_membership_admits_event("pk", "joined", "before-rejoin")
        .unwrap());
}

#[test]
fn automatic_body_eligibility_requires_arrival_and_signed_time_fences() {
    let store = Store::open_memory().unwrap();
    running(&store);
    insert_chat(&store, "future-seen-before", "joined", 500);
    store.grant_session_route("pk", "joined", 100).unwrap();
    insert_chat(&store, "same-second", "joined", 100);
    insert_chat(&store, "backdated", "joined", 99);
    insert_chat(&store, "wrong-channel", "other", 101);

    assert!(store
        .session_membership_admits_event("pk", "joined", "same-second")
        .unwrap());
    for event_id in [
        "future-seen-before",
        "backdated",
        "wrong-channel",
        "missing",
    ] {
        assert!(
            !store
                .session_membership_admits_event("pk", "joined", event_id)
                .unwrap(),
            "{event_id} must fail closed"
        );
    }

    insert_chat(&store, "future-seen-before", "joined", 500);
    assert!(!store
        .session_membership_admits_event("pk", "joined", "future-seen-before")
        .unwrap());
}

#[test]
fn persisted_join_fence_survives_restart_and_nmp_view_rebuild() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    {
        let store = Store::open(&path).unwrap();
        running(&store);
        insert_chat(&store, "before", "joined", 500);
        store.grant_session_route("pk", "joined", 100).unwrap();
        insert_chat(&store, "after", "joined", 100);
        assert!(!store
            .session_membership_admits_event("pk", "joined", "before")
            .unwrap());
        assert!(store
            .session_membership_admits_event("pk", "joined", "after")
            .unwrap());
    }

    let reopened = Store::open(&path).unwrap();
    insert_chat(&reopened, "before", "joined", 500);
    insert_chat(&reopened, "after", "joined", 100);
    assert!(!reopened
        .session_membership_admits_event("pk", "joined", "before")
        .unwrap());
    assert!(reopened
        .session_membership_admits_event("pk", "joined", "after")
        .unwrap());
}

fn route_fence(store: &Store, pubkey: &str, channel_h: &str) -> (u64, u64) {
    store
        .conn
        .query_row(
            "SELECT joined_at, joined_event_seq
               FROM session_channels
              WHERE pubkey=?1 AND channel_h=?2",
            params![pubkey, channel_h],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn insert_chat(store: &Store, id: &str, channel_h: &str, created_at: u64) {
    let mut events = store
        .nmp_views
        .events_by_kind(crate::fabric::nip29::wire::KIND_CHAT, u32::MAX);
    events.retain(|event| event.id != id);
    events.push(RelayEvent {
        id: id.into(),
        kind: 9,
        pubkey: "human".into(),
        created_at,
        channel_h: channel_h.into(),
        d_tag: String::new(),
        content: id.into(),
        tags_json: "[]".into(),
    });
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events(events));
}
