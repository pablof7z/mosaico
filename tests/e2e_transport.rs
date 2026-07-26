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
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, TagKind};
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
            workspace: String::new(),
            branch: String::new(),
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

#[tokio::test]
#[ignore = "requires the exact external MDB_NOLOCK Croissant binary"]
async fn croissant_admits_chat_by_sender_membership_not_p_tag_target_route() {
    let relay = TestRelay::start_nip29_relay();
    let admin_keys = Keys::generate();
    let member_keys = Keys::generate();
    let outsider_keys = Keys::generate();
    let unrouted_target = Keys::generate().public_key().to_hex();
    let group = format!(
        "sender-admission-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let admin = relay_client(&relay.url, admin_keys.clone()).await;
    let member = relay_client(&relay.url, member_keys.clone()).await;
    let outsider = relay_client(&relay.url, outsider_keys.clone()).await;

    let create = EventBuilder::new(Kind::from(9007u16), "")
        .tags([h_tag(&group)])
        .sign_with_keys(&admin_keys)
        .unwrap();
    assert!(!admin.send_event(&create).await.unwrap().success.is_empty());
    let lock = EventBuilder::new(Kind::from(9002u16), "")
        .tags([
            h_tag(&group),
            Tag::custom(TagKind::Custom("name".into()), [group.clone()]),
            Tag::custom(TagKind::Custom("closed".into()), Vec::<String>::new()),
            Tag::custom(TagKind::Custom("public".into()), Vec::<String>::new()),
        ])
        .sign_with_keys(&admin_keys)
        .unwrap();
    assert!(!admin.send_event(&lock).await.unwrap().success.is_empty());
    let put_member = EventBuilder::new(Kind::from(9000u16), "")
        .tags([
            h_tag(&group),
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P)),
                [member_keys.public_key().to_hex(), "member".into()],
            ),
        ])
        .sign_with_keys(&admin_keys)
        .unwrap();
    assert!(!admin
        .send_event(&put_member)
        .await
        .unwrap()
        .success
        .is_empty());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let roster = admin
        .fetch_events(
            Filter::new().kind(Kind::from(39002u16)).identifier(&group),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    let roster_pubkeys = roster
        .iter()
        .flat_map(|event| event.tags.iter())
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("p"))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
        .collect::<BTreeSet<_>>();
    assert!(roster_pubkeys.contains(&member_keys.public_key().to_hex()));
    assert!(!roster_pubkeys.contains(&unrouted_target));

    let direct_tags = || [h_tag(&group), p_tag(&unrouted_target)];
    let member_chat = EventBuilder::new(Kind::from(9u16), "member accepted")
        .tags(direct_tags())
        .sign_with_keys(&member_keys)
        .unwrap();
    let outsider_chat = EventBuilder::new(Kind::from(9u16), "outsider rejected")
        .tags(direct_tags())
        .sign_with_keys(&outsider_keys)
        .unwrap();
    let member_outcome = member.send_event(&member_chat).await.unwrap();
    let outsider_outcome = outsider.send_event(&outsider_chat).await.unwrap();

    assert!(
        !member_outcome.success.is_empty(),
        "member chat was not accepted: {:?}",
        member_outcome.failed
    );
    assert!(
        outsider_outcome.success.is_empty() && !outsider_outcome.failed.is_empty(),
        "nonmember chat unexpectedly passed: success={:?} failed={:?}",
        outsider_outcome.success,
        outsider_outcome.failed
    );

    admin.disconnect().await;
    member.disconnect().await;
    outsider.disconnect().await;
}

fn h_tag(group: &str) -> Tag {
    Tag::custom(
        TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::H)),
        [group],
    )
}

fn p_tag(pubkey: &str) -> Tag {
    Tag::custom(
        TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P)),
        [pubkey],
    )
}

async fn relay_client(relay: &str, keys: Keys) -> NmpRelayClient {
    NmpRelayClient::connect(keys, relay)
        .await
        .expect("connect NMP relay client")
}
