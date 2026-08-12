use super::*;

#[test]
fn projection_uses_lifecycle_transition_time_instead_of_lease_times() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
        crate::state::TestGroup::new("root").metadata("root", "", "", 1),
    ]));
    let pubkey = Keys::generate().public_key().to_hex();
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: pubkey.clone(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            launch_channel_h: "root".into(),
            work_root: "root".into(),
            child_pid: Some(42),
            now: 10,
        })
        .unwrap();
    let mut status = crate::state::Status {
        pubkey: pubkey.clone(),
        channel_h: "root".into(),
        slug: "codex".into(),
        title: "Picker status".into(),
        activity: String::new(),
        workspace: String::new(),
        branch: String::new(),
        state: crate::session_state::SessionState::Suspended,
        state_since: 10,
        last_seen: 20,
        updated_at: 20,
        expiration: 200,
    };
    status.last_seen = 100;
    status.updated_at = 100;
    store
        .install_test_nmp_relay_delivery(crate::state::TestRelayDelivery::new().statuses([status]));
    let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
    assert_eq!(rows[0]["state"], "suspended");
    assert_eq!(rows[0]["state_since"], 10);
}

#[test]
fn native_failure_is_separate_from_canonical_presence() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
        crate::state::TestGroup::new("root").metadata("root", "", "", 1),
    ]));
    let pubkey = Keys::generate().public_key().to_hex();
    let generation = store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: pubkey.clone(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            launch_channel_h: "root".into(),
            work_root: "root".into(),
            child_pid: Some(42),
            now: 10,
        })
        .unwrap();
    let attempt = store
        .start_native_turn_attempt(&crate::state::NewNativeTurnAttempt {
            pubkey: &pubkey,
            runtime_generation: generation,
            delivery_kind: crate::state::NativeTurnDeliveryKind::InboxEvent,
            delivery_event_id: "event",
            native_thread_id: "thread",
            started_at: 20,
        })
        .unwrap();
    store
        .finish_native_turn_attempt(&crate::state::FinishNativeTurnAttempt {
            id: attempt,
            pubkey: &pubkey,
            runtime_generation: generation,
            native_turn_id: "turn",
            outcome: crate::state::NativeTurnOutcome::Failed,
            error_message: "unsupported model",
            error_details: "",
            finished_at: 21,
        })
        .unwrap();

    let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
    assert_eq!(rows[0]["state"], "suspended");
    assert_eq!(rows[0]["native_outcome"]["outcome"], "failed");
    assert_eq!(rows[0]["native_outcome"]["delivery_kind"], "inbox_event");
    assert_eq!(
        rows[0]["native_outcome"]["error_message"],
        "unsupported model"
    );
}
