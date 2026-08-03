use super::*;

fn event(id: &str, created_at: u64) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: "pk".into(),
        created_at,
        channel_h: "h1".into(),
        d_tag: String::new(),
        content: String::new(),
        tags_json: "[]".into(),
    }
}

/// mosaico#744. A kind:5 retraction or a NIP-40 expiry reaches Mosaico as
/// `RowDelta::Removed(id)`; before this the delta was discarded and the row
/// stayed cached forever.
#[test]
fn retracting_an_event_removes_exactly_that_row() {
    let store = Store::open_memory().unwrap();
    assert!(store.insert_event(&event("a", 10)).unwrap());
    assert!(store.insert_event(&event("b", 11)).unwrap());

    assert!(store.retract_event("a").unwrap());
    assert!(store.get_event("a").unwrap().is_none());
    assert!(store.get_event("b").unwrap().is_some());

    // Retracting a row that is already gone is not an error: a supersession
    // handled by `insert_event` and reported again as `Removed` in the same
    // frame must not look like a failure.
    assert!(!store.retract_event("a").unwrap());
}

#[test]
fn chat_for_channel_after_preserves_same_second_id_cursor() {
    let store = Store::open_memory().unwrap();
    assert!(store.insert_event(&event("a", 10)).unwrap());
    assert!(store.insert_event(&event("b", 10)).unwrap());
    assert!(store.insert_event(&event("c", 11)).unwrap());

    let rows = store.chat_for_channel_after("h1", 10, "a", 10).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["b", "c"]
    );

    let rows = store.chat_for_channel_after("h1", 10, "b", 10).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["c"]
    );
}

#[test]
fn latest_message_at_by_pubkey_groups_by_author_max() {
    let store = Store::open_memory().unwrap();
    let mut pk_a1 = event("a1", 10);
    pk_a1.pubkey = "pk-a".into();
    let mut pk_a2 = event("a2", 30);
    pk_a2.pubkey = "pk-a".into();
    let mut pk_b1 = event("b1", 20);
    pk_b1.pubkey = "pk-b".into();
    // A non-chat kind must not count towards activity presence.
    let mut other_kind = event("k", 99);
    other_kind.pubkey = "pk-a".into();
    other_kind.kind = 7;
    for ev in [&pk_a1, &pk_a2, &pk_b1, &other_kind] {
        assert!(store.insert_event(ev).unwrap());
    }

    let latest = store.latest_message_at_by_pubkey("h1").unwrap();
    assert_eq!(latest.get("pk-a").copied(), Some(30));
    assert_eq!(latest.get("pk-b").copied(), Some(20));
    assert_eq!(latest.len(), 2);
    assert!(store
        .latest_message_at_by_pubkey("other")
        .unwrap()
        .is_empty());
}
