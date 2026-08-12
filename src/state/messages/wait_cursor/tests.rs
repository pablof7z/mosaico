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
fn wait_cursor_follows_nmp_arrival_order() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([event("first", "[]")]));
    let cursor = store.latest_message_arrival_sequence().unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().events([event("first", "[]"), event("second", "[]")]),
    );

    let rows = store.messages_after_arrival_sequence(cursor, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.message_id, "second");
}

#[test]
fn reply_target_comes_from_the_observed_message_tags() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([event(
        "reply",
        r#"[["e","root","","root"],["e","parent","","reply"]]"#,
    )]));
    let message = store.get_message("reply").unwrap().unwrap();
    assert_eq!(
        store.message_reply_target(&message).unwrap().as_deref(),
        Some("parent")
    );
}
