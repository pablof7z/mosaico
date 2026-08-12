use super::*;
use crate::state::{RegisterSession, Status, TestRelayDelivery};

fn local_session(store: &Store) {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: "local-pk".into(),
            observed_harness: "codex".into(),
            agent_slug: "local-codex".into(),
            launch_channel_h: "room".into(),
            work_root: "room".into(),
            child_pid: None,
            now: 10,
        })
        .unwrap();
}

fn remote_status(state: SessionState, expiration: u64) -> Status {
    Status {
        pubkey: "remote-pk".into(),
        channel_h: "room".into(),
        slug: "remote-codex".into(),
        title: String::new(),
        activity: String::new(),
        workspace: String::new(),
        branch: String::new(),
        state,
        state_since: 10,
        last_seen: 10,
        updated_at: 10,
        expiration,
    }
}

#[test]
fn suspended_local_recipient_gets_manual_resumption_reminder() {
    let store = Store::open_memory().unwrap();
    local_session(&store);

    let recipients = vec![TaggedRecipient {
        label: "local-codex".into(),
        pubkey: "local-pk".into(),
        channel: "room".into(),
    }];
    let reminders = suspension_reminders(&store, &recipients).unwrap();

    assert_eq!(
        reminders,
        vec![
            "Reminder: @local-codex is suspended and will receive this message after manual resumption."
        ]
    );
    let reminder = &reminders[0];
    for private_mechanic in ["PTY", "ACP", "endpoint", "supervisor", "backend"] {
        assert!(!reminder.contains(private_mechanic));
    }
}

#[test]
fn suspended_reply_author_gets_the_same_reminder_contract() {
    let store = Store::open_memory().unwrap();
    local_session(&store);
    let original = Message {
        message_id: "message".into(),
        channel_h: "room".into(),
        author_pubkey: "local-pk".into(),
        body: "hello".into(),
        created_at: 9,
        attachment_dir: String::new(),
    };

    assert_eq!(
        reply_suspension_reminders(&store, &original).unwrap(),
        vec![
            "Reminder: @local-codex is suspended and will receive this message after manual resumption."
        ]
    );
}

#[test]
fn working_and_offline_local_recipients_do_not_get_reminders() {
    let store = Store::open_memory().unwrap();
    local_session(&store);
    let generation = store
        .get_session("local-pk")
        .unwrap()
        .unwrap()
        .runtime_generation;
    store
        .apply_session_turn_started("local-pk", generation, 11)
        .unwrap();
    assert!(suspension_reminder(&store, "local-pk", "room", None)
        .unwrap()
        .is_none());

    store
        .mark_runtime_stopped("local-pk", crate::state::StopReason::HeadlessExit, 12)
        .unwrap();
    assert!(suspension_reminder(&store, "local-pk", "room", None)
        .unwrap()
        .is_none());
}

#[test]
fn fresh_peer_state_controls_the_reminder() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().statuses([remote_status(SessionState::Suspended, 20)]),
    );
    assert!(suspension_reminder(&store, "remote-pk", "room", None)
        .unwrap()
        .is_some());

    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().statuses([remote_status(SessionState::Idle, 30)]),
    );
    assert!(suspension_reminder(&store, "remote-pk", "room", None)
        .unwrap()
        .is_none());
}

#[test]
fn current_peer_row_is_not_reinterpreted_by_a_second_expiry_clock() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().statuses([remote_status(SessionState::Suspended, 9)]),
    );

    assert!(suspension_reminder(&store, "remote-pk", "room", None)
        .unwrap()
        .is_some());
}
