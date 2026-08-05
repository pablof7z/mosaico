use super::*;

fn record(id: &str) -> RecordMessage {
    record_at(id, "author-pk", "accepted", 10)
}

fn record_at(id: &str, author_pubkey: &str, sync_state: &str, created_at: u64) -> RecordMessage {
    RecordMessage {
        message_id: id.to_string(),
        thread_id: "chan".to_string(),
        channel_h: "chan".to_string(),
        author_pubkey: author_pubkey.to_string(),
        body: "hello".to_string(),
        created_at,
        sync_state: sync_state.to_string(),
        native_event_id: Some(id.to_string()),
        error: None,
    }
}

#[test]
fn relay_replay_cannot_erase_or_replace_materialized_attachment_directory() {
    let store = Store::open_memory().unwrap();
    store.record_message(&record("event-files")).unwrap();
    assert!(store
        .set_message_attachment_dir(
            "event-files",
            std::path::Path::new("/tmp/mosaico-files/abcdef"),
        )
        .unwrap());
    assert!(!store
        .set_message_attachment_dir(
            "event-files",
            std::path::Path::new("/tmp/mosaico-files/replacement"),
        )
        .unwrap());
    store.record_message(&record("event-files")).unwrap();

    assert_eq!(
        store
            .get_message("event-files")
            .unwrap()
            .unwrap()
            .attachment_dir,
        "/tmp/mosaico-files/abcdef"
    );
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
            tags_json: r#"[["p","recipient-pk"]]"#.to_string(),
        })
        .unwrap();

    store.backfill_messages_from_relay_events().unwrap();

    let msg = store.get_message("event-2").unwrap().unwrap();
    assert_eq!(msg.author_pubkey, "author-pk");
    assert_eq!(msg.body, "from relay");
    assert_eq!(
        store.message_recipients("event-2").unwrap()[0].recipient_pubkey,
        "recipient-pk"
    );
}

#[test]
fn recent_channel_messages_limit_keeps_the_newest_rows() {
    let store = Store::open_memory().unwrap();
    for (id, at) in [("old", 10), ("middle", 20), ("new", 30)] {
        store
            .record_message(&record_at(id, "inbound", "accepted", at))
            .unwrap();
    }

    let rows = store
        .recent_chat_messages_for_channel("chan", 0, 2)
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|message| message.message_id.as_str())
            .collect::<Vec<_>>(),
        ["new", "middle"]
    );
}

#[test]
fn latest_channel_activity_uses_only_accepted_messages() {
    let store = Store::open_memory().unwrap();
    for (id, state, at) in [
        ("old", "accepted", 10),
        ("failed", "failed", 30),
        ("latest", "accepted", 20),
    ] {
        store
            .record_message(&record_at(id, "inbound", state, at))
            .unwrap();
    }

    assert_eq!(
        store.latest_accepted_message_at_by_channel().unwrap()["chan"],
        20
    );
}

/// The reply-nudge check asks whether THIS agent has spoken in the channel
/// since the mention, and `author_pubkey` is the whole answer. A local
/// `direction` column used to be conjoined with it and could only ever agree,
/// because every caller passes its own key.
#[test]
fn the_reply_check_follows_the_authoring_pubkey_and_the_accepted_state() {
    let store = Store::open_memory().unwrap();
    store
        .record_message(&record_at("older", "author-pk", "accepted", 99))
        .unwrap();
    store
        .record_message(&record_at("someone-else", "other-pk", "accepted", 101))
        .unwrap();
    store
        .record_message(&record_at("not-accepted", "author-pk", "failed", 102))
        .unwrap();

    assert!(!store
        .pubkey_has_own_message_after_in_channel("author-pk", "chan", 100)
        .unwrap());

    store
        .record_message(&record_at("mine", "author-pk", "accepted", 101))
        .unwrap();

    assert!(store
        .pubkey_has_own_message_after_in_channel("author-pk", "chan", 100)
        .unwrap());
    // A different channel is a different conversation.
    assert!(!store
        .pubkey_has_own_message_after_in_channel("author-pk", "other-chan", 100)
        .unwrap());
}

#[test]
fn recipient_edge_is_unique_per_pubkey_and_keeps_latest_delivery() {
    let store = Store::open_memory().unwrap();
    store.record_message(&record("event-3")).unwrap();
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
