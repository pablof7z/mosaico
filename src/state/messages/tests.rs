use super::*;
use crate::state::TestRelayDelivery;

fn event(id: &str, author: &str, channel: &str, body: &str, at: u64) -> RelayEvent {
    RelayEvent {
        id: id.to_string(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: author.to_string(),
        created_at: at,
        channel_h: channel.to_string(),
        d_tag: String::new(),
        content: body.to_string(),
        tags_json: "[]".to_string(),
    }
}

#[test]
fn attachment_can_arrive_before_fava_observes_the_message() {
    let store = Store::open_memory().unwrap();
    assert!(store
        .set_message_attachment_dir(
            "event-files",
            std::path::Path::new("/tmp/mosaico-files/abcdef"),
        )
        .unwrap());
    assert!(store.get_message("event-files").unwrap().is_none());
    assert!(!store
        .set_message_attachment_dir(
            "event-files",
            std::path::Path::new("/tmp/mosaico-files/replacement"),
        )
        .unwrap());

    store.install_test_relay_delivery(TestRelayDelivery::new().events([event(
        "event-files",
        "author",
        "chan",
        "hello",
        10,
    )]));
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
fn message_reads_and_channel_activity_follow_the_current_fava_delivery() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("old", "author", "chan", "old", 10),
        event("middle", "author", "chan", "middle", 20),
        event("new", "author", "chan", "new", 30),
    ]));

    let rows = store
        .recent_chat_messages_for_channel("chan", 0, 2)
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|message| message.message_id.as_str())
            .collect::<Vec<_>>(),
        ["new", "middle"]
    );
    assert_eq!(store.latest_message_at_by_channel().unwrap()["chan"], 30);

    store.install_test_relay_delivery(TestRelayDelivery::new());
    assert!(store.get_message("new").unwrap().is_none());
    assert!(store.latest_message_at_by_channel().unwrap().is_empty());
}

#[test]
fn p_tags_are_the_recipient_projection() {
    let store = Store::open_memory().unwrap();
    let mut mentioned = event("event", "author", "chan", "hello", 10);
    mentioned.tags_json = r#"[["p","z"],["p","a"],["p","z"]]"#.to_string();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([mentioned]));

    let recipients = store.message_recipients("event").unwrap();
    assert_eq!(
        recipients
            .iter()
            .map(|recipient| recipient.recipient_pubkey.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
}

#[test]
fn reply_check_uses_only_observed_authorship() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("other", "other", "chan", "other", 101),
        event("old", "author", "chan", "old", 99),
    ]));
    assert!(!store
        .pubkey_has_own_message_after_in_channel("author", "chan", 100)
        .unwrap());

    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("other", "other", "chan", "other", 101),
        event("mine", "author", "chan", "mine", 101),
    ]));
    assert!(store
        .pubkey_has_own_message_after_in_channel("author", "chan", 100)
        .unwrap());
}

#[test]
fn get_message_by_prefix_resolves_unique_and_rejects_ambiguous() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("mine", "author", "chan", "m", 10),
        event("middle", "author", "chan", "m", 20),
    ]));
    // "min" matches only "mine" ("middle" does not start with "min").
    assert_eq!(
        store
            .get_message_by_prefix("min")
            .unwrap()
            .unwrap()
            .message_id,
        "mine"
    );
    // "mi" matches both → ambiguous error.
    assert!(store
        .get_message_by_prefix("mi")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(store.get_message_by_prefix("zzz").unwrap().is_none());
}

#[test]
fn chat_messages_for_channel_filters_and_orders_ascending() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("m1", "a", "chan", "x", 10),
        event("m2", "a", "chan", "x", 20),
        event("o", "a", "other", "x", 15),
    ]));
    let ids = |v: Vec<_>| {
        v.into_iter()
            .map(|m: super::Message| m.message_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(store
            .chat_messages_for_channel("chan", 5, u32::MAX)
            .unwrap()),
        vec!["m1", "m2"]
    );
    assert_eq!(
        ids(store
            .chat_messages_for_channel("chan", 15, u32::MAX)
            .unwrap()),
        vec!["m2"]
    );
    assert_eq!(
        ids(store.chat_messages_for_channel("chan", 5, 1).unwrap()),
        vec!["m1"]
    );
    assert_eq!(
        ids(store
            .chat_messages_for_channel("other", 0, u32::MAX)
            .unwrap()),
        vec!["o"]
    );
    assert!(store
        .chat_messages_for_channel("nope", 0, u32::MAX)
        .unwrap()
        .is_empty());
}

#[test]
fn chat_messages_for_channel_after_uses_created_at_then_id_cursor() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("a", "x", "chan", "x", 10),
        event("b", "x", "chan", "x", 20),
        event("c", "x", "chan", "x", 20), // same second as b, id "c" > "b"
        event("d", "x", "chan", "x", 30),
    ]));
    let ids = |v: Vec<_>| {
        v.into_iter()
            .map(|m: super::Message| m.message_id)
            .collect::<Vec<_>>()
    };
    // after (20, "b"): created_at>20 OR (==20 AND id>"b") → c, d (ascending).
    assert_eq!(
        ids(store
            .chat_messages_for_channel_after("chan", 20, "b", u32::MAX)
            .unwrap()),
        vec!["c", "d"]
    );
    // after (10, "a"): everything strictly after a → b, c, d.
    assert_eq!(
        ids(store
            .chat_messages_for_channel_after("chan", 10, "a", u32::MAX)
            .unwrap()),
        vec!["b", "c", "d"]
    );
}

#[test]
fn recent_chat_messages_is_global_recent_first() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("a", "x", "chan1", "x", 10),
        event("b", "x", "chan2", "x", 20),
    ]));
    let ids = |v: Vec<_>| {
        v.into_iter()
            .map(|m: super::Message| m.message_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(store.recent_chat_messages(0, u32::MAX).unwrap()),
        vec!["b", "a"]
    );
    assert_eq!(
        ids(store.recent_chat_messages(15, u32::MAX).unwrap()),
        vec!["b"]
    );
    assert_eq!(ids(store.recent_chat_messages(0, 1).unwrap()), vec!["b"]);
}

#[test]
fn should_render_reply_nudge_requires_no_self_reaction_and_no_followup() {
    let store = Store::open_memory().unwrap();
    // msg by A at t10; A follows up with later at t20; no self-reaction.
    store.install_test_relay_delivery(TestRelayDelivery::new().events([
        event("msg", "A", "chan", "q", 10),
        event("later", "A", "chan", "followup", 20),
    ]));
    // A has its own message after t15 → no nudge.
    assert!(!store
        .should_render_reply_nudge("chan", "msg", "A", 15)
        .unwrap());

    // Fresh store: msg by A, no follow-up, no self-reaction → nudge.
    let store2 = Store::open_memory().unwrap();
    store2.install_test_relay_delivery(
        TestRelayDelivery::new().events([event("msg", "A", "chan", "q", 10)]),
    );
    assert!(store2
        .should_render_reply_nudge("chan", "msg", "A", 15)
        .unwrap());

    // Self-reaction by A on its own msg suppresses the nudge even with no follow-up.
    let store3 = Store::open_memory().unwrap();
    store3.install_test_relay_delivery(TestRelayDelivery::new().events([
        RelayEvent {
            id: "msg".into(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: "A".into(),
            created_at: 10,
            channel_h: "chan".into(),
            d_tag: String::new(),
            content: "q".into(),
            tags_json: "[]".into(),
        },
        RelayEvent {
            id: "react".into(),
            kind: 7,
            pubkey: "A".into(),
            created_at: 12,
            channel_h: "chan".into(),
            d_tag: String::new(),
            content: "👍".into(),
            tags_json: r#"[["e","msg"]]"#.to_string(),
        },
    ]));
    assert!(
        !store3
            .should_render_reply_nudge("chan", "msg", "A", 15)
            .unwrap(),
        "a self-reaction suppresses the nudge even with no follow-up"
    );
}
