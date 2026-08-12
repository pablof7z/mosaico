use super::*;
use nostr::{EventBuilder, Kind, Tag};
use std::sync::Arc;
use std::time::Duration;

mod auth_harness;
mod boot_recovery;
use auth_harness::AuthRequiredRelay;

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
fn canonical_nmp_view_stream_has_exactly_one_owner() {
    let host = NmpHost::open(&[], None, None, &Keys::generate()).unwrap();
    let receiver = host.take_view_transitions().unwrap();
    assert!(host.take_view_transitions().is_err());
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
    let subscription = host
        .observe_with_access(
            &SubscriptionQuery::Kinds {
                kinds: BTreeSet::from([9000]),
            },
            AccessContext::Nip42(backend.public_key()),
        )
        .expect("open authenticated read");
    let acquired = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || loop {
            let frame = subscription
                .recv()
                .expect("authenticated observation remains open");
            if let Some(event) = frame.deltas.iter().find_map(|delta| delta.event().cloned()) {
                break event;
            }
        }),
    )
    .await
    .expect("authenticated read deadline")
    .expect("authenticated observation task");
    assert_eq!(acquired.id, seed.id);

    // Acceptance is immediate and says nothing about the relay; the relay's
    // own observation is what proves the authenticated session carried the
    // event, so that is what is waited on.
    let written = host
        .publish_group(
            "auth-room",
            EventBuilder::new(Kind::TextNote, "authenticated agent write"),
            &agent,
        )
        .expect("NMP takes custody of the authenticated write");

    let observation = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let observation = relay.observation();
            if observation
                .ordinary_events
                .iter()
                .any(|event| event.id == written)
            {
                break observation;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("strict relay accepts the authenticated write");
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
