use super::*;

fn record(id: &str, direction: &str) -> RecordMessage {
    record_at(id, direction, "accepted", 10)
}

fn record_at(id: &str, direction: &str, sync_state: &str, created_at: u64) -> RecordMessage {
    RecordMessage {
        message_id: id.to_string(),
        thread_id: "chan".to_string(),
        channel_h: "chan".to_string(),
        author_pubkey: "author-pk".to_string(),
        body: "hello".to_string(),
        created_at,
        direction: direction.to_string(),
        sync_state: sync_state.to_string(),
        native_event_id: Some(id.to_string()),
        error: None,
    }
}

#[test]
fn relay_replay_preserves_local_outbound_direction() {
    let store = Store::open_memory().unwrap();
    store
        .record_message(&record("event-1", "outbound"))
        .unwrap();
    store.record_message(&record("event-1", "inbound")).unwrap();

    let msg = store.get_message("event-1").unwrap().unwrap();
    assert_eq!(msg.author_pubkey, "author-pk");
    assert_eq!(msg.direction, "outbound");
}

#[test]
fn relay_event_backfill_uses_event_author_pubkey() {
    let store = Store::open_memory().unwrap();
    store
        .insert_event(&RelayEvent {
            id: "event-2".to_string(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: "author-pk".to_string(),
            created_at: 10,
            channel_h: "chan".to_string(),
            d_tag: String::new(),
            content: "from relay".to_string(),
            tags_json: "[]".to_string(),
        })
        .unwrap();

    store.backfill_messages_from_relay_events().unwrap();

    let msg = store.get_message("event-2").unwrap().unwrap();
    assert_eq!(msg.author_pubkey, "author-pk");
    assert_eq!(msg.body, "from relay");
}

#[test]
fn outbound_reply_check_follows_pubkey_across_runtime_replacement() {
    let store = Store::open_memory().unwrap();
    store
        .record_message(&record_at("old-outbound", "outbound", "accepted", 99))
        .unwrap();
    store
        .record_message(&record_at("inbound", "inbound", "accepted", 101))
        .unwrap();
    store
        .record_message(&record_at("failed-outbound", "outbound", "failed", 102))
        .unwrap();

    assert!(!store
        .pubkey_has_outbound_message_since("author-pk", 100)
        .unwrap());

    store
        .record_message(&record_at("accepted-outbound", "outbound", "accepted", 100))
        .unwrap();

    assert!(store
        .pubkey_has_outbound_message_since("author-pk", 100)
        .unwrap());
    assert!(!store
        .pubkey_has_outbound_message_since("other-pk", 100)
        .unwrap());
}

#[test]
fn recipient_edge_is_unique_per_pubkey_and_keeps_latest_delivery() {
    let store = Store::open_memory().unwrap();
    store
        .record_message(&record("event-3", "outbound"))
        .unwrap();
    store
        .add_message_recipient("event-3", "recipient-pk", None)
        .unwrap();
    store
        .add_message_recipient("event-3", "recipient-pk", Some(42))
        .unwrap();
    store
        .add_message_recipient("event-3", "recipient-pk", Some(30))
        .unwrap();

    let rows = store.message_recipients("event-3").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].recipient_pubkey, "recipient-pk");
    assert_eq!(rows[0].delivered_at, Some(42));
}
