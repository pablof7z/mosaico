use super::*;
use crate::state::{RegisterSession, Store};
use nostr::{EventBuilder, Keys, Kind, Tag};

fn make_tag(parts: &[&str]) -> Tag {
    Tag::parse(parts.iter().copied()).unwrap()
}

fn build(keys: &Keys, body: &str, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::from(9u16), body.to_string())
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

fn build_at(keys: &Keys, body: &str, tags: Vec<Tag>, created_at: u64) -> Event {
    EventBuilder::new(Kind::from(9u16), body.to_string())
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from_secs(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn chat(sender_pk: &str, channel: &str, body: &str, mention: Option<String>) -> ChatMessage {
    ChatMessage {
        from: AgentRef::new(sender_pk, String::new()),
        channel: channel.to_string(),
        body: body.to_string(),
        mentioned_pubkeys: mention.into_iter().collect(),
    }
}

fn register(store: &Store, pubkey: &str, channel: &str, agent_slug: &str) {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "codex".into(),
            agent_slug: agent_slug.into(),
            launch_channel_h: channel.into(),
            work_root: channel.into(),
            child_pid: None,
            now: 1,
        })
        .unwrap();
}

#[test]
fn materialize_chat_quarantines_until_membership_snapshots_hydrate() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let receiver = Keys::generate();
    let sender_pk = sender.public_key().to_hex();
    let receiver_pk = receiver.public_key().to_hex();
    register(&store, &receiver_pk, "proj", "receiver");

    let event = build(
        &sender,
        "ship it",
        vec![make_tag(&["h", "proj"]), make_tag(&["p", &receiver_pk])],
    );
    let chat = chat(&sender_pk, "proj", "ship it", Some(receiver_pk.clone()));

    let out = materialize_chat(&store, &event, &chat);
    assert!(!out.wake_mentions);
    assert_eq!(store.count_quarantined_events("proj").unwrap(), 1);
    assert!(!store.has_event(&event.id.to_hex()).unwrap());
    assert!(store.get_message(&event.id.to_hex()).unwrap().is_none());

    assert!(!replay_quarantined_chat(&store, "proj"));
    assert_eq!(store.count_quarantined_events("proj").unwrap(), 1);

    store
        .replace_channel_admins("proj", &Vec::<String>::new(), 10)
        .unwrap();
    store
        .replace_channel_members("proj", &[sender_pk, receiver_pk.clone()], 11)
        .unwrap();

    assert!(replay_quarantined_chat(&store, "proj"));
    assert_eq!(store.count_quarantined_events("proj").unwrap(), 0);
    assert!(store.has_event(&event.id.to_hex()).unwrap());
    assert_eq!(
        store
            .get_message(&event.id.to_hex())
            .unwrap()
            .unwrap()
            .sync_state,
        "accepted"
    );
    let pending = store.peek_pending_for_pubkey(&receiver_pk).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].body, "ship it");
}

#[test]
fn materialize_chat_rejects_non_member_after_hydration() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let sender_pk = sender.public_key().to_hex();
    let event = build(&sender, "not admitted", vec![make_tag(&["h", "proj"])]);
    let chat = chat(&sender_pk, "proj", "not admitted", None);

    store
        .replace_channel_admins("proj", &Vec::<String>::new(), 10)
        .unwrap();
    store
        .replace_channel_members("proj", &Vec::<String>::new(), 11)
        .unwrap();

    let out = materialize_chat(&store, &event, &chat);
    assert!(out.tail.is_none());
    assert!(!out.wake_mentions);
    assert_eq!(store.count_quarantined_events("proj").unwrap(), 0);
    assert!(!store.has_event(&event.id.to_hex()).unwrap());
    assert!(store.get_message(&event.id.to_hex()).unwrap().is_none());
}

#[test]
fn replay_preserves_the_true_prejoin_arrival_fence() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let receiver = Keys::generate();
    let sender_pk = sender.public_key().to_hex();
    let receiver_pk = receiver.public_key().to_hex();
    let event = build_at(
        &sender,
        "future-dated prejoin history",
        vec![make_tag(&["h", "proj"]), make_tag(&["p", &receiver_pk])],
        10_000,
    );
    let chat = chat(
        &sender_pk,
        "proj",
        "future-dated prejoin history",
        Some(receiver_pk.clone()),
    );

    materialize_chat(&store, &event, &chat);
    register(&store, &receiver_pk, "proj", "receiver");
    store
        .replace_channel_admins("proj", &Vec::<String>::new(), 10)
        .unwrap();
    store
        .replace_channel_members("proj", &[sender_pk, receiver_pk.clone()], 11)
        .unwrap();

    assert!(!replay_quarantined_chat(&store, "proj"));
    assert!(store.has_event(&event.id.to_hex()).unwrap());
    assert!(!store
        .session_membership_admits_event(&receiver_pk, "proj", &event.id.to_hex())
        .unwrap());
    assert!(store
        .peek_pending_for_pubkey(&receiver_pk)
        .unwrap()
        .is_empty());
}
