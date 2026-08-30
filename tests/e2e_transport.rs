//! End-to-end coverage for Nostr codec and Fava acquisition boundaries.

#[path = "common/mod.rs"]
mod common;
#[path = "common/fava_client.rs"]
mod fava_client;

use common::TestRelay;
use mosaico::domain::{AgentRef, DomainEvent, Profile, Status};
use mosaico::fabric::nip29::wire::Nip29WireCodec;
use fava_client::FavaRelayClient;
use nostr::event::{EventBuilder, Kind, Tag};
use nostr::filter::Filter;
use nostr::key::Keys;
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
async fn fava_acquires_from_an_explicitly_allowed_local_relay() {
    use fava::RelayUrl;
    use fava_query::Query;

    let relay = TestRelay::start();
    let author = Keys::generate();
    let relay_url = RelayUrl::parse(&relay.url).expect("valid relay URL");

    // Build a fava instance pinned to the local relay.
    let fava = {
        use std::sync::Arc;
        use fava::FavaBuilder;
        use fava_delivery_standard::StandardDeliveryPolicy;
        use fava_event_cache_memory::MemoryEventCache;
        use fava_publisher_nip01::Nip01Publisher;
        use fava_query_standard::StandardQueryEvaluator;
        use fava_router_app_relays::AppRelayRouter;
        use fava_signer_local::LocalSigner;
        use fava_subscriptions_no_grouping::planner;
        use fava_transport_websocket::WebSocketTransport;
        use fava_write_store_memory::MemoryWriteStore;
        FavaBuilder::default()
            .event_cache(Arc::new(MemoryEventCache::default()))
            .query_evaluator(Arc::new(StandardQueryEvaluator))
            .subscription_planner(Arc::new(planner()))
            .transport(Arc::new(WebSocketTransport::new()))
            .router(Arc::new(AppRelayRouter::new("app", [relay_url.clone()])))
            .signer(Arc::new(LocalSigner::new(author.clone())))
            .publisher(Arc::new(Nip01Publisher))
            .delivery_policy(Arc::new(StandardDeliveryPolicy::default()))
            .write_store(Arc::new(MemoryWriteStore::default()))
            .build()
            .expect("fava build")
    };

    let query = Query::events()
        .authors([author.public_key()])
        .expect("bounded author set")
        .kinds([Kind::TextNote])
        .expect("bounded kind set")
        .only_from_relays([relay_url])
        .expect("relay set");
    let mut observation = fava.observe(query).await.expect("fava observes");

    let writer = relay_client(&relay.url, author).await;
    writer
        .send_event_builder(EventBuilder::new(Kind::TextNote, "hello from Fava"))
        .await
        .expect("publish");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut found = false;
    while !found && std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, observation.changed()).await {
            Ok(Ok(snapshot)) => {
                found = snapshot.events.iter().any(|rec| {
                    if let fava::EventValue::Signed(e) = rec.event() {
                        e.content == "hello from Fava"
                    } else {
                        false
                    }
                });
            }
            _ => break,
        }
    }
    observation.close();
    assert!(found, "fava did not acquire the published event");
}

#[tokio::test]
async fn bounded_fava_read_accepts_an_active_empty_acquisition() {
    let relay = TestRelay::start();
    let client = relay_client(&relay.url, Keys::generate()).await;
    let events = client
        .fetch_events(
            Filter::new().kind(Kind::from(65_535u16)),
            Duration::from_secs(5),
        )
        .await
        .expect("empty Fava read should complete with acquisition evidence");
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

    use nostr::event::FinalizeEvent as _;
    let create = EventBuilder::new(Kind::from(9007u16), "")
        .tags([h_tag(&group)])
        .finalize(&admin_keys)
        .unwrap();
    assert!(!admin.send_event(&create).await.unwrap().success.is_empty());
    let lock = EventBuilder::new(Kind::from(9002u16), "")
        .tags([
            h_tag(&group),
            Tag::custom("name", [group.clone()]),
            Tag::custom("closed", Vec::<String>::new()),
            Tag::custom("public", Vec::<String>::new()),
        ])
        .finalize(&admin_keys)
        .unwrap();
    assert!(!admin.send_event(&lock).await.unwrap().success.is_empty());
    let put_member = EventBuilder::new(Kind::from(9000u16), "")
        .tags([
            h_tag(&group),
            Tag::custom("p", [member_keys.public_key().to_hex(), "member".into()]),
        ])
        .finalize(&admin_keys)
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
        .finalize(&member_keys)
        .unwrap();
    let outsider_chat = EventBuilder::new(Kind::from(9u16), "outsider rejected")
        .tags(direct_tags())
        .finalize(&outsider_keys)
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
    Tag::custom("h", [group])
}

fn p_tag(pubkey: &str) -> Tag {
    Tag::custom("p", [pubkey])
}

async fn relay_client(relay: &str, keys: Keys) -> FavaRelayClient {
    FavaRelayClient::connect(keys, relay)
        .await
        .expect("connect Fava relay client")
}
