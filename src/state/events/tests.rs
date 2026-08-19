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

fn chat(id: &str, created_at: u64, channel: &str) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: "pk".into(),
        created_at,
        channel_h: channel.into(),
        d_tag: String::new(),
        content: String::new(),
        tags_json: "[]".into(),
    }
}

#[test]
fn get_event_by_prefix_returns_unique_and_rejects_ambiguous() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().events([chat("abc", 1, "h1"), chat("abe", 2, "h1")]),
    );
    // Unique prefix resolves to the one match.
    assert_eq!(store.get_event_by_prefix("abc").unwrap().unwrap().id, "abc");
    // Shared prefix matches more than one → error, not a silent pick.
    assert!(store
        .get_event_by_prefix("ab")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(store.get_event_by_prefix("zzz").unwrap().is_none());
    assert!(store.has_event("abc").unwrap());
    assert!(!store.has_event("zzz").unwrap());
}

#[test]
fn chat_for_channel_filters_by_channel_since_and_limit() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
        chat("c1", 10, "h1"),
        chat("c2", 20, "h1"),
        chat("c3", 15, "h2"),
    ]));
    let ids = |events: Vec<RelayEvent>| events.into_iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(
        ids(store.chat_for_channel("h1", 0, u32::MAX).unwrap()),
        vec!["c1", "c2"]
    );
    assert_eq!(
        ids(store.chat_for_channel("h1", 15, u32::MAX).unwrap()),
        vec!["c2"]
    );
    assert_eq!(ids(store.chat_for_channel("h1", 0, 1).unwrap()), vec!["c1"]);
    assert_eq!(
        ids(store.chat_for_channel("h2", 0, u32::MAX).unwrap()),
        vec!["c3"]
    );
    assert!(store
        .chat_for_channel("never", 0, u32::MAX)
        .unwrap()
        .is_empty());
}

#[test]
fn events_by_kind_returns_recent_first_and_filters_kind() {
    let store = Store::open_memory().unwrap();
    let mut reaction = chat("r1", 30, "h1");
    reaction.kind = 7;
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
        chat("a", 10, "h1"),
        chat("b", 20, "h1"),
        reaction,
    ]));
    let ids = |events: Vec<RelayEvent>| events.into_iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(
        ids(store.events_by_kind(9, u32::MAX).unwrap()),
        vec!["b", "a"]
    );
    assert_eq!(ids(store.events_by_kind(7, u32::MAX).unwrap()), vec!["r1"]);
    assert!(store.events_by_kind(99, u32::MAX).unwrap().is_empty());
}

#[test]
fn prejoin_chat_for_session_returns_chat_up_to_join_sequence_newest_first() {
    let store = Store::open_memory().unwrap();
    // The session joined at arrival sequence 2; only chats observed at or before
    // that fence are pre-join history for this session.
    store
        .conn
        .execute(
            "INSERT INTO session_channels (pubkey, channel_h, joined_at, joined_event_seq)
         VALUES ('pk-sess', 'h1', 1, 2)",
            [],
        )
        .unwrap();
    assert_eq!(store.record_nmp_arrival("m1").unwrap(), 1);
    assert_eq!(store.record_nmp_arrival("m2").unwrap(), 2);
    assert_eq!(store.record_nmp_arrival("m3").unwrap(), 3);

    let mut non_chat = chat("x1", 25, "h1");
    non_chat.kind = 7;
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
        chat("m1", 10, "h1"),
        chat("m2", 20, "h1"),
        chat("m3", 30, "h1"),
        non_chat,
    ]));

    let ids = |events: Vec<RelayEvent>| events.into_iter().map(|e| e.id).collect::<Vec<_>>();
    // seq 1,2 <= 2; m3 (seq 3) excluded; non-chat (seq 4? actually arrives in delivery order) excluded by kind.
    let prejoin = store
        .prejoin_chat_for_session("pk-sess", "h1", 100)
        .unwrap();
    assert_eq!(
        ids(prejoin),
        vec!["m2", "m1"],
        "newest-first, fenced by join sequence"
    );

    // Limit caps the result.
    assert_eq!(
        ids(store.prejoin_chat_for_session("pk-sess", "h1", 1).unwrap()),
        vec!["m2"]
    );

    // Unknown session → empty.
    assert!(store
        .prejoin_chat_for_session("nobody", "h1", 100)
        .unwrap()
        .is_empty());
}
