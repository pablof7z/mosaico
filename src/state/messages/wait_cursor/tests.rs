use super::*;

fn record(id: &str) -> RecordMessage {
    RecordMessage {
        message_id: id.into(),
        thread_id: "channel".into(),
        channel_h: "channel".into(),
        author_pubkey: "author".into(),
        author_session: None,
        body: id.into(),
        created_at: 1,
        direction: "inbound".into(),
        sync_state: "accepted".into(),
        native_event_id: Some(id.into()),
        error: None,
    }
}

#[test]
fn reply_target_prefers_marked_reply_and_falls_back_to_last_e_tag() {
    assert_eq!(
        reply_target_from_tags_json(r#"[["e","root","","root"],["e","parent","","reply"]]"#)
            .as_deref(),
        Some("parent")
    );
    assert_eq!(
        reply_target_from_tags_json(r#"[["e","root"],["e","parent"]]"#).as_deref(),
        Some("parent")
    );
    assert_eq!(reply_target_from_tags_json("[]"), None);
}

#[test]
fn rowid_cursor_returns_messages_in_local_arrival_order() {
    let store = Store::open_memory().unwrap();
    store.record_message(&record("first")).unwrap();
    let cursor = store.latest_message_rowid().unwrap();
    store.record_message(&record("second")).unwrap();

    let rows = store.messages_after_rowid(cursor, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.message_id, "second");
}
