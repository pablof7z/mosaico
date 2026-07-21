use super::*;
use nostr_sdk::prelude::{EventBuilder, Kind, Tag};
use std::sync::Arc;
use std::time::Duration;

mod auth_harness;
use auth_harness::AuthRequiredRelay;

#[test]
fn query_is_authenticated_as_the_backend_and_pinned_to_the_configured_host() {
    let relay = RelayUrl::parse("wss://relay.example.com").unwrap();
    let relays = BTreeSet::from([relay.clone()]);
    let backend = Keys::generate().public_key();
    let query = SubscriptionQuery {
        kinds: BTreeSet::from([9, 30315]),
        tag: Some(('h', "room".into())),
    };

    let live = live_query(&relays, &query, AccessContext::Nip42(backend)).unwrap();
    assert_eq!(live.0.access, AccessContext::Nip42(backend));
    assert_eq!(live.0.source, SourceAuthority::Pinned(relays));
    assert_eq!(live.0.selection.kinds, Some(query.kinds));
    let h = IndexedTagName::new('h').unwrap();
    assert_eq!(
        live.0.selection.tags.get(&h),
        Some(&Binding::Literal(BTreeSet::from(["room".to_string()])))
    );
}

#[test]
fn configured_local_hosts_are_explicitly_allowed_but_onion_is_not() {
    let local = RelayUrl::parse("ws://127.0.0.1:7777").unwrap();
    let public = RelayUrl::parse("wss://relay.example.com").unwrap();
    let onion = RelayUrl::parse("ws://examplehiddenservice.onion").unwrap();

    assert_eq!(
        local_relay_hosts([&local, &public, &onion]),
        vec!["127.0.0.1"]
    );
}

#[test]
fn canonical_materialization_stream_has_exactly_one_owner() {
    let host = NmpHost::open(&[], None, None, &Keys::generate()).unwrap();
    let receiver = host.take_materialization_events().unwrap();
    assert!(host.take_materialization_events().is_err());
    drop(receiver);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_relay_authenticates_backend_reads_and_exact_author_writes() {
    let backend = Keys::generate();
    let agent = Keys::generate();
    let seed = EventBuilder::new(Kind::from(9000u16), "")
        .tags([
            Tag::parse(["h", "auth-room"]).unwrap(),
            Tag::parse(["p", &agent.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let relay =
        AuthRequiredRelay::spawn([backend.public_key(), agent.public_key()], [seed.clone()]);
    let host = Arc::new(
        NmpHost::open(&[relay.url()], None, None, &backend).expect("open authenticated NMP host"),
    );
    let mut materialized = host
        .take_materialization_events()
        .expect("take materialization stream");

    host.apply(&SubEffect::Open {
        id: "nip42-capstone".into(),
        query: SubscriptionQuery {
            kinds: BTreeSet::from([9000]),
            tag: None,
        },
    })
    .expect("open authenticated read");
    let acquired = tokio::time::timeout(Duration::from_secs(10), materialized.recv())
        .await
        .expect("authenticated read deadline")
        .expect("materialization stream remains open");
    assert_eq!(acquired.id, seed.id);

    let written =
        tokio::time::timeout(
            Duration::from_secs(10),
            host.publish_group_builder(
                EventBuilder::new(Kind::TextNote, "authenticated agent write")
                    .tags([Tag::parse(["h", "auth-room"]).unwrap()]),
                &agent,
                true,
            ),
        )
        .await
        .expect("authenticated write deadline")
        .expect("strict relay accepts authenticated write");

    let observation = relay.observation();
    assert_eq!(observation.pre_auth_reqs, 0, "REQ escaped before AUTH");
    assert_eq!(observation.pre_auth_events, 0, "EVENT escaped before AUTH");
    assert!(
        observation.invalid_auth.is_empty(),
        "strict relay rejected AUTH: {:?}",
        observation.invalid_auth
    );
    assert!(
        observation
            .auth_events
            .iter()
            .any(|event| event.pubkey == backend.public_key()),
        "backend read identity never authenticated: {observation:?}"
    );
    assert!(
        observation
            .auth_events
            .iter()
            .any(|event| event.pubkey == agent.public_key()),
        "agent write identity never authenticated: {observation:?}"
    );
    assert!(
        observation
            .authenticated_reqs
            .iter()
            .any(|(pubkey, filters)| {
                *pubkey == backend.public_key()
                    && filters
                        .iter()
                        .any(|filter| filter.match_event(&seed, Default::default()))
            }),
        "no authenticated backend REQ matched the seeded event: {observation:?}"
    );
    assert!(
        observation
            .ordinary_events
            .iter()
            .any(|event| event.id == written && event.pubkey == agent.public_key()),
        "agent event did not cross the authenticated session: {observation:?}"
    );

    host.shutdown();
    relay.shutdown();
}
