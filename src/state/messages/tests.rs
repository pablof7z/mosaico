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
fn attachment_can_arrive_before_nmp_observes_the_message() {
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

    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([event(
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
fn message_reads_and_channel_activity_follow_the_current_nmp_delivery() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
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

    store.install_test_nmp_relay_delivery(TestRelayDelivery::new());
    assert!(store.get_message("new").unwrap().is_none());
    assert!(store.latest_message_at_by_channel().unwrap().is_empty());
}

#[test]
fn p_tags_are_the_recipient_projection() {
    let store = Store::open_memory().unwrap();
    let mut mentioned = event("event", "author", "chan", "hello", 10);
    mentioned.tags_json = r#"[["p","z"],["p","a"],["p","z"]]"#.to_string();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([mentioned]));

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
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
        event("other", "other", "chan", "other", 101),
        event("old", "author", "chan", "old", 99),
    ]));
    assert!(!store
        .pubkey_has_own_message_after_in_channel("author", "chan", 100)
        .unwrap());

    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
        event("other", "other", "chan", "other", 101),
        event("mine", "author", "chan", "mine", 101),
    ]));
    assert!(store
        .pubkey_has_own_message_after_in_channel("author", "chan", 100)
        .unwrap());
}
