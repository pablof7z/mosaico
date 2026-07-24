//! End-to-end coverage for Nostr codec and NMP acquisition boundaries.

#[path = "common/mod.rs"]
mod common;
#[path = "common/nmp_client.rs"]
mod nmp_client;

use common::TestRelay;
use mosaico::domain::{AgentRef, DomainEvent, Profile, Status};
use mosaico::fabric::nip29::wire::Nip29WireCodec;
use nmp::{
    AccessContext, Binding, Demand, Engine, EngineConfig, Filter as NmpFilter, LiveQuery, RelayUrl,
    SourceAuthority,
};
use nmp_client::NmpRelayClient;
use nostr::{EventBuilder, Filter, Keys, Kind};
use std::collections::BTreeSet;
use std::time::Duration;

#[tokio::test]
async fn publishes_and_decodes_all_event_types() {
    let relay = TestRelay::start();
    let codec = Nip29WireCodec;

    let agent_keys = Keys::generate();
    let reader_keys = Keys::generate();
    let agent_pubkey = agent_keys.public_key();
    let agent_pk = agent_pubkey.to_hex();
    let reader_pk = reader_keys.public_key().to_hex();
    let channel = "mosaico".to_string();

    let agent = relay_client(&relay.url, agent_keys).await;
    let aref = AgentRef::new(agent_pk.clone(), "coder");

    let events = vec![
        DomainEvent::Profile(Profile {
            agent: aref.clone(),
            agent_slug: "coder".into(),
            host: "test-host".into(),
            workspace: channel.clone(),
            owners: vec![reader_pk.clone()],
            is_backend: false,
            agents: Vec::new(),
            workspaces: Vec::new(),
        }),
        DomainEvent::Status(Status {
            agent: aref.clone(),
            channels: vec![channel.clone()],
            host: "test-host".into(),
            title: "fixing the auth bug".into(),
            activity: "reading the diff".into(),
            state: mosaico::session_state::SessionState::Working,
            state_since: 1_800_000_000,
            rel_cwd: String::new(),
            expires_at: Some(1_900_000_000),
            dispatch_event: None,
        }),
    ];
    for ev in &events {
        let builder = codec.encode_event(ev).expect("encode");
        agent.send_event_builder(builder).await.expect("publish");
    }

    let fetched = agent
        .fetch_events(
            Filter::new()
                .author(agent_pubkey)
                .kinds([Kind::from(0), Kind::from(30315)]),
            Duration::from_secs(5),
        )
        .await
        .expect("fetch published events");
    let seen: Vec<DomainEvent> = fetched
        .iter()
        .filter_map(|event| codec.decode_event(event))
        .collect();

    // Identify the status by its title; the decoded status also carries its
    // session id, but the title is the stable user-facing session summary.
    let has_status = seen
        .iter()
        .any(|e| matches!(e, DomainEvent::Status(s) if s.title == "fixing the auth bug"));
    let has_profile = seen
        .iter()
        .any(|e| matches!(e, DomainEvent::Profile(p) if p.host == "test-host"));
    assert!(has_status, "expected status; saw {seen:#?}");
    assert!(has_profile, "expected profile; saw {seen:#?}");
}

#[tokio::test]
async fn nmp_acquires_from_an_explicitly_allowed_local_relay() {
    let relay = TestRelay::start();
    let author = Keys::generate();
    let author_hex = author.public_key().to_hex();
    let relay_url = RelayUrl::parse(&relay.url).expect("valid relay URL");
    let engine = Engine::new(EngineConfig {
        app_relays: vec![relay.url.clone()],
        allowed_local_relay_hosts: vec!["127.0.0.1".into()],
        ..EngineConfig::default()
    })
    .expect("NMP engine starts");
    let query = LiveQuery(
        Demand::new(
            NmpFilter {
                kinds: Some(BTreeSet::from([1])),
                authors: Some(Binding::Literal(BTreeSet::from([author_hex]))),
                ..NmpFilter::default()
            },
            SourceAuthority::Pinned(BTreeSet::from([relay_url])),
            AccessContext::Public,
        )
        .expect("valid pinned demand"),
    );
    let subscription = engine.observe(query, None).expect("NMP observes");
    let writer = relay_client(&relay.url, author).await;
    writer
        .send_event_builder(EventBuilder::text_note("hello from NMP"))
        .await
        .expect("publish");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut found = false;
    while !found && std::time::Instant::now() < deadline {
        if let Ok(frame) = subscription.recv_timeout(Duration::from_millis(500)) {
            found = frame
                .deltas
                .iter()
                .filter_map(|delta| delta.event())
                .any(|event| event.content == "hello from NMP");
        }
    }
    assert!(found, "NMP did not acquire the published event");
    engine.shutdown();
}

#[tokio::test]
async fn bounded_nmp_read_accepts_an_active_empty_acquisition() {
    let relay = TestRelay::start();
    let client = relay_client(&relay.url, Keys::generate()).await;
    let events = client
        .fetch_events(
            Filter::new().kind(Kind::from(65_535u16)),
            Duration::from_secs(5),
        )
        .await
        .expect("empty NMP read should complete with acquisition evidence");
    assert!(events.is_empty());
}

async fn relay_client(relay: &str, keys: Keys) -> NmpRelayClient {
    NmpRelayClient::connect(keys, relay)
        .await
        .expect("connect NMP relay client")
}
