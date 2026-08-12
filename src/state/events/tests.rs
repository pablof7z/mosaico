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

/// A later NMP delivery omitting a removed row immediately changes reads.
#[test]
fn retracting_an_event_removes_exactly_that_row() {
    let store = Store::open_memory().unwrap();
    let a = event("a", 10);
    let b = event("b", 11);
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([a, b.clone()]));

    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([b]));
    assert!(store.get_event("a").unwrap().is_none());
    assert!(store.get_event("b").unwrap().is_some());
}

#[test]
fn chat_for_channel_after_preserves_same_second_id_cursor() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
        event("a", 10),
        event("b", 10),
        event("c", 11),
    ]));

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
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().events([pk_a1, pk_a2, pk_b1, other_kind]),
    );

    let latest = store.latest_message_at_by_pubkey("h1").unwrap();
    assert_eq!(latest.get("pk-a").copied(), Some(30));
    assert_eq!(latest.get("pk-b").copied(), Some(20));
    assert_eq!(latest.len(), 2);
    assert!(store
        .latest_message_at_by_pubkey("other")
        .unwrap()
        .is_empty());
}
