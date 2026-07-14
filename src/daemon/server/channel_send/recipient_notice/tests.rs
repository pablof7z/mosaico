use super::*;
use crate::state::{RegisterSession, Status};

fn local_session(store: &Store) {
    store
        .reserve_session(&RegisterSession {
            pubkey: "local-pk".into(),
            harness: "codex".into(),
            agent_slug: "local-codex".into(),
            channel_h: "room".into(),
            child_pid: None,
            transcript_path: None,
            now: 10,
        })
        .unwrap();
}

fn remote_status(store: &Store, state: SessionState, expiration: u64) {
    store
        .upsert_status(&Status {
            pubkey: "remote-pk".into(),
            channel_h: "room".into(),
            slug: "remote-codex".into(),
            title: String::new(),
            activity: String::new(),
            state,
            last_seen: 10,
            updated_at: 10,
            expiration,
        })
        .unwrap();
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
    let reminders = suspension_reminders(&store, &recipients, 10).unwrap();

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
        thread_id: "thread".into(),
        channel_h: "room".into(),
        author_pubkey: "local-pk".into(),
        body: "hello".into(),
        created_at: 9,
        direction: "inbound".into(),
        sync_state: "published".into(),
        native_event_id: Some("event".into()),
        error: None,
    };

    assert_eq!(
        reply_suspension_reminders(&store, &original, 10).unwrap(),
        vec![
            "Reminder: @local-codex is suspended and will receive this message after manual resumption."
        ]
    );
}

#[test]
fn working_and_offline_local_recipients_do_not_get_reminders() {
    let store = Store::open_memory().unwrap();
    local_session(&store);
    store.set_working("local-pk", true, 11).unwrap();
    assert!(suspension_reminder(&store, "local-pk", "room", None, 11)
        .unwrap()
        .is_none());

    store.mark_dead("local-pk").unwrap();
    assert!(suspension_reminder(&store, "local-pk", "room", None, 12)
        .unwrap()
        .is_none());
}

#[test]
fn fresh_peer_state_controls_the_reminder() {
    let store = Store::open_memory().unwrap();
    remote_status(&store, SessionState::Suspended, 20);
    assert!(suspension_reminder(&store, "remote-pk", "room", None, 10,)
        .unwrap()
        .is_some());

    remote_status(&store, SessionState::Idle, 30);
    assert!(suspension_reminder(&store, "remote-pk", "room", None, 10,)
        .unwrap()
        .is_none());
}

#[test]
fn expired_peer_suspension_is_observed_as_offline() {
    let store = Store::open_memory().unwrap();
    remote_status(&store, SessionState::Suspended, 9);

    assert!(suspension_reminder(&store, "remote-pk", "room", None, 10,)
        .unwrap()
        .is_none());
}
