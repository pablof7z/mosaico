use super::*;
use crate::state::{RegisterSession, Store};
use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

fn make_tag(parts: &[&str]) -> Tag {
    Tag::parse(parts.iter().copied()).unwrap()
}

fn build(keys: &Keys, kind_n: u16, content: &str, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::from(kind_n), content)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

fn register(store: &Store, pubkey: &str, channel_h: &str, external_id: &str) -> String {
    store
        .register_session(&RegisterSession {
            harness: "claude-code".into(),
            external_id_kind: "harness_session".into(),
            external_id: external_id.into(),
            agent_pubkey: pubkey.into(),
            agent_slug: "agent".into(),
            channel_h: channel_h.into(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now: 100,
        })
        .unwrap()
}

#[test]
fn channel_metadata_materializes() {
    let store = Store::open_memory().unwrap();
    let relay = Keys::generate();
    let event = build(
        &relay,
        39000,
        "",
        vec![
            make_tag(&["d", "proj"]),
            make_tag(&["name", "Project"]),
            make_tag(&["about", "the thing"]),
            make_tag(&["parent", ""]),
        ],
    );
    Nip29Materializer::materialize_channel(&store, &event);
    let ch = store.get_channel("proj").unwrap().unwrap();
    assert_eq!(ch.name, "Project");
    assert_eq!(ch.about, "the thing");
    assert!(store.is_root_channel("proj").unwrap());
}

#[test]
fn admins_and_members_preserve_each_other() {
    let store = Store::open_memory().unwrap();
    let relay = Keys::generate();
    let admin = Keys::generate().public_key().to_hex();
    let member = Keys::generate().public_key().to_hex();

    let admins = build(
        &relay,
        39001,
        "",
        vec![make_tag(&["d", "proj"]), make_tag(&["p", &admin])],
    );
    let members = build(
        &relay,
        39002,
        "",
        vec![make_tag(&["d", "proj"]), make_tag(&["p", &member])],
    );
    Nip29Materializer::materialize_admins(&store, &admins);
    Nip29Materializer::materialize_members(&store, &members);

    assert!(store.is_channel_admin("proj", &admin).unwrap());
    assert!(store.is_channel_member("proj", &member).unwrap());
    assert!(!store.is_channel_admin("proj", &member).unwrap());
}

#[test]
fn profile_materializes_to_relay_profiles() {
    let store = Store::open_memory().unwrap();
    let agent = Keys::generate();
    let pk = agent.public_key().to_hex();
    let event = build(
        &agent,
        0,
        r#"{"name":"smith"}"#,
        vec![make_tag(&["host", "laptop"])],
    );
    let de = crate::fabric::nip29::wire::Nip29WireCodec.decode_event(&event);
    if let Some(crate::domain::DomainEvent::Profile(pf)) = de {
        Nip29Materializer::materialize_profile(&store, &pf, event.created_at.as_secs());
    }
    assert_eq!(
        store.resolve_slug_for_pubkey(&pk).unwrap().as_deref(),
        Some("smith")
    );
}

#[test]
fn status_materializes_and_reads_live() {
    let store = Store::open_memory().unwrap();
    let agent = Keys::generate();
    let pk = agent.public_key().to_hex();
    let exp = 10_000u64;
    let event = build(
        &agent,
        30315,
        "compiling",
        vec![
            make_tag(&["d", "proj"]),
            make_tag(&["h", "proj"]),
            make_tag(&["title", "build"]),
            make_tag(&["status", "busy"]),
            make_tag(&["slug", "smith"]),
            make_tag(&["expiration", &exp.to_string()]),
        ],
    );
    let de = crate::fabric::nip29::wire::Nip29WireCodec.decode_event(&event);
    if let Some(crate::domain::DomainEvent::Status(st)) = de {
        Nip29Materializer::materialize_status(&store, &st, event.created_at.as_secs());
    }
    let raw = store.get_status(&pk, "", "proj").unwrap().unwrap();
    assert_eq!(raw.title, "build");
    assert!(raw.busy);
    assert_eq!(
        store
            .live_status_for_channel("proj", exp - 1)
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .live_status_for_channel("proj", exp + 1)
        .unwrap()
        .is_empty());
}

#[test]
fn chat_routes_to_channel_sessions_and_skips_sender() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let receiver = Keys::generate();
    let sender_pk = sender.public_key().to_hex();
    let receiver_pk = receiver.public_key().to_hex();

    let sender_sid = register(&store, &sender_pk, "proj", "sender-ext");
    let receiver_sid = register(&store, &receiver_pk, "proj", "receiver-ext");

    // Without a p-tag the message is ambient chat: stored in relay_events
    // but does NOT route to any inbox (no doorbell).
    let ambient_event = build(&sender, 9, "ambient", vec![make_tag(&["h", "proj"])]);
    let ambient_chat = ChatMessage {
        from: crate::domain::AgentRef::new(sender_pk.clone(), String::new()),
        project: "proj".into(),
        body: "ambient".into(),
        mentioned_pubkey: None,
    };
    assert!(Nip29Materializer::materialize_event(&store, &ambient_event));
    assert!(!Nip29Materializer::route_chat(
        &store,
        &ambient_event,
        &ambient_chat
    ));
    assert!(store
        .drain_pending_for_session(&receiver_sid)
        .unwrap()
        .is_empty());

    // With a p-tag the message is a directed mention: routed to inbox.
    let mention_event = build(
        &sender,
        9,
        "ship it",
        vec![make_tag(&["h", "proj"]), make_tag(&["p", &receiver_pk])],
    );
    let mention_chat = ChatMessage {
        from: crate::domain::AgentRef::new(sender_pk, String::new()),
        project: "proj".into(),
        body: "ship it".into(),
        mentioned_pubkey: Some(receiver_pk),
    };
    assert!(Nip29Materializer::materialize_event(&store, &mention_event));
    assert!(Nip29Materializer::route_chat(
        &store,
        &mention_event,
        &mention_chat
    ));

    let pending = store.drain_pending_for_session(&receiver_sid).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].body, "ship it");
    assert!(store
        .drain_pending_for_session(&sender_sid)
        .unwrap()
        .is_empty());
    assert!(store.has_event(&mention_event.id.to_hex()).unwrap());
}

/// Two concurrent sessions of the SAME agent slug but DIFFERENT ordinal
/// pubkeys (ordinal 0 and ordinal 1) must route independently: a mention
/// p-tagging only ordinal 0's pubkey reaches ONLY that session, never the
/// sibling ordinal. Regression for the double-delivery bug where every
/// ordinal of an agent shared the base pubkey, so one mention woke both.
#[test]
fn mention_to_one_ordinal_does_not_route_to_sibling_ordinal() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let ord0 = Keys::generate(); // ordinal 0 (base) pubkey
    let ord1 = Keys::generate(); // ordinal 1 (HKDF-derived) pubkey — distinct
    let sender_pk = sender.public_key().to_hex();
    let ord0_pk = ord0.public_key().to_hex();
    let ord1_pk = ord1.public_key().to_hex();

    // Both sessions are the same agent slug ("agent") in the same channel.
    let ord0_sid = register(&store, &ord0_pk, "proj", "ord0-ext");
    let ord1_sid = register(&store, &ord1_pk, "proj", "ord1-ext");

    // Mention p-tags ONLY ordinal 0.
    let event = build(
        &sender,
        9,
        "hey ordinal zero",
        vec![make_tag(&["h", "proj"]), make_tag(&["p", &ord0_pk])],
    );
    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(sender_pk, String::new()),
        project: "proj".into(),
        body: "hey ordinal zero".into(),
        mentioned_pubkey: Some(ord0_pk),
    };
    assert!(Nip29Materializer::route_chat(&store, &event, &chat));

    assert_eq!(
        store.drain_pending_for_session(&ord0_sid).unwrap().len(),
        1,
        "the p-tagged ordinal must receive the mention"
    );
    assert!(
        store
            .drain_pending_for_session(&ord1_sid)
            .unwrap()
            .is_empty(),
        "the sibling ordinal must NOT receive a mention addressed to ordinal 0"
    );
}

#[test]
fn other_kind_lands_in_relay_events() {
    let store = Store::open_memory().unwrap();
    let agent = Keys::generate();
    let event = build(&agent, 1, "a social note", vec![make_tag(&["h", "proj"])]);
    assert!(Nip29Materializer::materialize_event(&store, &event));
    let stored = store.get_event(&event.id.to_hex()).unwrap().unwrap();
    assert_eq!(stored.kind, 1);
    assert_eq!(stored.channel_h, "proj");
    assert_eq!(stored.content, "a social note");
}
