use super::*;
use crate::state::{RelayEvent, TestRelayDelivery};

fn event(id: &str, tags_json: &str) -> RelayEvent {
    RelayEvent {
        id: id.to_string(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: "author".to_string(),
        created_at: 1,
        channel_h: "channel".to_string(),
        d_tag: String::new(),
        content: id.to_string(),
        tags_json: tags_json.to_string(),
    }
}

#[test]
fn wait_cursor_follows_relay_arrival_order() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([event("first", "[]")]));
    let cursor = store.latest_message_arrival_sequence().unwrap();
    store.install_test_relay_delivery(
        TestRelayDelivery::new().events([event("first", "[]"), event("second", "[]")]),
    );

    let rows = store
        .messages_after_arrival_sequence(cursor, 10)
        .unwrap()
        .messages;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.message_id, "second");
}

#[test]
fn reply_target_comes_from_the_observed_message_tags() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([event(
        "reply",
        r#"[["e","root","","root"],["e","parent","","reply"]]"#,
    )]));
    let message = store.get_message("reply").unwrap().unwrap();
    assert_eq!(
        store.message_reply_target(&message).unwrap().as_deref(),
        Some("parent")
    );
}

#[test]
fn wait_cursor_returns_chat_in_arrival_order_and_stops_at_the_limit() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([event("c", "[]")]));
    store.record_event_arrival("ghost").unwrap();
    let mut reaction = event("r", r#"[["e","c"]]"#);
    reaction.kind = crate::fabric::nip29::wire::KIND_REACTION as u32;
    reaction.content = "+".to_string();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("a", "[]"),
        event("b", "[]"),
        event("c", "[]"),
        reaction,
    ]));

    let ids = |limit| {
        store
            .messages_after_arrival_sequence(0, limit)
            .unwrap()
            .messages
            .into_iter()
            .map(|(_, message)| message.message_id)
            .collect::<Vec<_>>()
    };
    // Arrival order (c first), not view order; the ghost id and the reaction
    // are arrivals but not chat messages in the view.
    assert_eq!(ids(10), ["c", "a", "b"]);
    assert_eq!(
        ids(2),
        ["c", "a"],
        "skipped arrivals do not count toward the limit"
    );
    assert_eq!(ids(1), ["c"]);
    assert!(ids(0).is_empty());
    // With no limit hit, the batch reports the last arrival scanned (the
    // reaction), so a reader that matched nothing can skip past it.
    let latest = store.latest_message_arrival_sequence().unwrap();
    let batch = store.messages_after_arrival_sequence(0, 10).unwrap();
    assert_eq!(batch.scanned_through, latest);
}
