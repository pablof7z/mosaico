use super::*;

#[test]
fn semantic_status_change_is_a_delta_without_resetting_state_age() {
    let store = seed_store();
    let rec = session(&store);
    let mut peer = Status {
        pubkey: OTHER_PK.into(),
        channel_h: "root".into(),
        slug: "amber-reviewer".into(),
        title: "Reviewing".into(),
        activity: String::new(),
        workspace: "root".into(),
        branch: String::new(),
        state: crate::session_state::SessionState::Suspended,
        state_since: 90,
        last_seen: 90,
        updated_at: 90,
        expiration: 240,
    };
    peer.title = "Updated title".into();
    peer.last_seen = 150;
    peer.updated_at = 150;
    peer.expiration = 300;
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([peer]),
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 100, 160, true))
        .expect("status-change delta should render");
    assert!(text.contains("<members>"), "got: {text}");
    assert!(!text.contains("<recent-presence>"), "got: {text}");
    assert!(text.contains("status=\"Updated title\""), "got: {text}");
    assert!(text.contains("since=\"1 min ago\""), "got: {text}");
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 100, 160, true)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 100, 160)),
        text
    );
}

#[test]
fn native_failure_delta_does_not_overwrite_presence_state() {
    let store = seed_store();
    let rec = session(&store);
    let generation = store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: OTHER_PK.into(),
            observed_harness: "codex".into(),
            agent_slug: "reviewer".into(),
            launch_channel_h: "root".into(),
            work_root: "root".into(),
            child_pid: None,
            now: 90,
        })
        .unwrap();
    let attempt = store
        .start_native_turn_attempt(&crate::state::NewNativeTurnAttempt {
            pubkey: OTHER_PK,
            runtime_generation: generation,
            delivery_kind: crate::state::NativeTurnDeliveryKind::InboxEvent,
            delivery_event_id: "event",
            native_thread_id: "thread",
            started_at: 140,
        })
        .unwrap();
    store
        .finish_native_turn_attempt(&crate::state::FinishNativeTurnAttempt {
            id: attempt,
            pubkey: OTHER_PK,
            runtime_generation: generation,
            native_turn_id: "turn",
            outcome: crate::state::NativeTurnOutcome::Failed,
            error_message: "unsupported model",
            error_details: "",
            finished_at: 150,
        })
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 100, 160, true))
        .expect("native failure delta should render");
    assert!(text.contains("<native-outcome"), "got: {text}");
    assert!(text.contains("outcome=\"failed\""), "got: {text}");
    assert!(text.contains("text=\"unsupported model\""), "got: {text}");
    assert!(!text.contains("state=\"failed\""), "got: {text}");
}
