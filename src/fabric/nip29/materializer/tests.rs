use super::*;
use crate::state::{RegisterSession, Store};
use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag, Timestamp, ToBech32};

mod agent_roster;
mod membership;
mod repeated_tags;

fn make_tag(parts: &[&str]) -> Tag {
    Tag::parse(parts.iter().copied()).unwrap()
}

fn build(keys: &Keys, kind_n: u16, content: &str, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::from(kind_n), content)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

fn build_at(keys: &Keys, kind_n: u16, content: &str, tags: Vec<Tag>, created_at: u64) -> Event {
    EventBuilder::new(Kind::from(kind_n), content)
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn register(store: &Store, pubkey: &str, channel_h: &str, agent_slug: &str) {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "claude-code".into(),
            agent_slug: agent_slug.into(),
            channel_h: channel_h.into(),
            child_pid: None,
            transcript_path: None,
            now: 100,
        })
        .unwrap();
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
            make_tag(&["name", "Channel"]),
            make_tag(&["about", "the thing"]),
            make_tag(&["parent", ""]),
        ],
    );
    Nip29Materializer::materialize_channel(&store, &event);
    let ch = store.get_channel("proj").unwrap().unwrap();
    assert_eq!(ch.name, "Channel");
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
        r#"{"name":"willow-echo-042"}"#,
        vec![
            make_tag(&["host", "laptop"]),
            make_tag(&["agent-slug", "developer"]),
        ],
    );
    let de = crate::fabric::nip29::wire::Nip29WireCodec.decode_event(&event);
    if let Some(crate::domain::DomainEvent::Profile(pf)) = de {
        Nip29Materializer::materialize_profile(&store, &pf, event.created_at.as_secs());
    }
    assert_eq!(
        store.resolve_slug_for_pubkey(&pk).unwrap().as_deref(),
        Some("willow-echo-042-developer")
    );
    let profile = store.get_profile(&pk).unwrap().unwrap();
    assert_eq!(profile.name, "willow-echo-042-developer");
    assert_eq!(profile.slug, "willow-echo-042-developer");
    assert_eq!(profile.agent_slug, "developer");
    assert!(
        store
            .resolve_profile_handle_pubkey("willow-echo-042-developer")
            .unwrap()
            .is_none(),
        "profile names alone are not lease authority"
    );
}

#[test]
fn retired_profile_materializes_npub_without_recreating_handle() {
    let store = Store::open_memory().unwrap();
    let agent = Keys::generate();
    let pk = agent.public_key().to_hex();
    let npub = agent.public_key().to_bech32().unwrap();
    let event = build(
        &agent,
        0,
        &serde_json::json!({ "name": npub }).to_string(),
        vec![
            make_tag(&["host", "laptop"]),
            make_tag(&["agent-slug", "developer"]),
        ],
    );
    let Some(crate::domain::DomainEvent::Profile(profile)) =
        crate::fabric::nip29::wire::Nip29WireCodec.decode_event(&event)
    else {
        panic!("expected profile");
    };
    Nip29Materializer::materialize_profile(&store, &profile, event.created_at.as_secs());

    let cached = store.get_profile(&pk).unwrap().unwrap();
    assert_eq!(cached.name, npub);
    assert_eq!(cached.slug, npub);
    assert_eq!(cached.agent_slug, "developer");
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
            make_tag(&["d", "status"]),
            make_tag(&["h", "proj"]),
            make_tag(&["title", "build"]),
            make_tag(&["state", "working"]),
            make_tag(&["state-since", "42"]),
            make_tag(&["host", "laptop"]),
            make_tag(&["slug", "smith"]),
            make_tag(&["expiration", &exp.to_string()]),
        ],
    );
    let de = crate::fabric::nip29::wire::Nip29WireCodec.decode_event(&event);
    if let Some(crate::domain::DomainEvent::Status(st)) = de {
        Nip29Materializer::materialize_status(&store, &st, event.created_at.as_secs());
    }
    let raw = store.get_status(&pk, "proj").unwrap().unwrap();
    assert_eq!(raw.title, "build");
    assert_eq!(raw.state, crate::session_state::SessionState::Working);
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

    register(&store, &sender_pk, "proj", "sender-ext");
    register(&store, &receiver_pk, "proj", "receiver-ext");

    // Without a p-tag the message is ambient chat: stored in relay_events
    // but does NOT route to any inbox (no doorbell).
    let ambient_event = build(&sender, 9, "ambient", vec![make_tag(&["h", "proj"])]);
    let ambient_chat = ChatMessage {
        from: crate::domain::AgentRef::new(sender_pk.clone(), String::new()),
        channel: "proj".into(),
        body: "ambient".into(),
        mentioned_pubkeys: Vec::new(),
    };
    assert!(Nip29Materializer::materialize_event(&store, &ambient_event));
    assert!(!Nip29Materializer::route_chat(
        &store,
        &ambient_event,
        &ambient_chat
    ));
    assert!(store
        .peek_pending_for_pubkey(&receiver_pk)
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
        from: crate::domain::AgentRef::new(sender_pk.clone(), String::new()),
        channel: "proj".into(),
        body: "ship it".into(),
        mentioned_pubkeys: vec![receiver_pk.clone()],
    };
    assert!(Nip29Materializer::materialize_event(&store, &mention_event));
    assert!(Nip29Materializer::route_chat(
        &store,
        &mention_event,
        &mention_chat
    ));

    let pending = store.peek_pending_for_pubkey(&receiver_pk).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].body, "ship it");
    assert!(store
        .peek_pending_for_pubkey(&sender_pk)
        .unwrap()
        .is_empty());
    assert!(store.has_event(&mention_event.id.to_hex()).unwrap());
}

/// Two concurrent sessions of the SAME agent slug but DIFFERENT ordinal pubkeys
/// must route independently: a mention p-tagging only one ordinal's pubkey
/// reaches ONLY that session, never the sibling ordinal.
#[test]
fn mention_to_one_ordinal_does_not_route_to_sibling_ordinal() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let ord0 = Keys::generate();
    let ord1 = Keys::generate();
    let sender_pk = sender.public_key().to_hex();
    let ord0_pk = ord0.public_key().to_hex();
    let ord1_pk = ord1.public_key().to_hex();

    // Both sessions are the same agent slug ("agent") in the same channel.
    register(&store, &ord0_pk, "proj", "ord0-ext");
    register(&store, &ord1_pk, "proj", "ord1-ext");

    // Mention p-tags ONLY one ordinal.
    let event = build(
        &sender,
        9,
        "hey one ordinal",
        vec![make_tag(&["h", "proj"]), make_tag(&["p", &ord0_pk])],
    );
    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(sender_pk, String::new()),
        channel: "proj".into(),
        body: "hey one ordinal".into(),
        mentioned_pubkeys: vec![ord0_pk.clone()],
    };
    assert!(Nip29Materializer::route_chat(&store, &event, &chat));

    assert_eq!(
        store.peek_pending_for_pubkey(&ord0_pk).unwrap().len(),
        1,
        "the p-tagged ordinal must receive the mention"
    );
    assert!(
        store.peek_pending_for_pubkey(&ord1_pk).unwrap().is_empty(),
        "the sibling ordinal must NOT receive a mention addressed to another ordinal"
    );
}

#[test]
fn mention_to_dead_session_stays_pending_for_that_exact_pubkey() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let target = Keys::generate();
    let sibling = Keys::generate();
    let target_pk = target.public_key().to_hex();
    let sibling_pk = sibling.public_key().to_hex();

    register(&store, &target_pk, "proj", "target-ext");
    register(&store, &sibling_pk, "proj", "sibling-ext");
    store
        .mark_runtime_stopped(&target_pk, crate::state::StopReason::HeadlessExit, 1)
        .unwrap();
    assert!(
        !store.is_channel_member("proj", &target_pk).unwrap(),
        "relay membership is not the durable exact-resume affinity"
    );

    let event = build(
        &sender,
        9,
        "resume this exact session",
        vec![make_tag(&["h", "proj"]), make_tag(&["p", &target_pk])],
    );
    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(sender.public_key().to_hex(), String::new()),
        channel: "proj".into(),
        body: "resume this exact session".into(),
        mentioned_pubkeys: vec![target_pk.clone()],
    };

    assert!(Nip29Materializer::route_chat(&store, &event, &chat));
    assert_eq!(store.peek_pending_for_pubkey(&target_pk).unwrap().len(), 1);
    assert!(store
        .peek_pending_for_pubkey(&sibling_pk)
        .unwrap()
        .is_empty());
    assert!(!store.get_session(&target_pk).unwrap().unwrap().is_running());
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
