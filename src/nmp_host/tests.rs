use super::*;
use nmp::{Binding, CacheMode, IndexedTagName, SourceAuthority};
use nostr::{EventBuilder, Kind, Tag};
use std::sync::Arc;
use std::time::Duration;

mod auth_harness;
use auth_harness::AuthRequiredRelay;

const HOST_A: &str = "wss://a.example.com";
const HOST_B: &str = "wss://b.example.com";

fn two_host_host() -> NmpHost {
    NmpHost::open(
        &[HOST_A.to_string(), HOST_B.to_string()],
        None,
        None,
        &Keys::generate(),
    )
    .expect("open NMP host over two group relays")
}

fn pinned_to(url: &str) -> SourceAuthority {
    SourceAuthority::Pinned(BTreeSet::from([RelayUrl::parse(url).unwrap()]))
}

/// mosaico#741. Pinning scopes only which relays are ASKED; `CacheMode`
/// governs which locally cached rows may ANSWER, and its default is
/// `Agnostic` — "every matching cached row regardless of provenance". A
/// NIP-29 read needs both axes pointed at the same host, because 39000/39001/
/// 39002 are relay-SIGNED and two relays hosting one group id are two
/// independent groups.
///
/// This replaces a test that asserted the previous shape: ONE branch pinned
/// to the whole relay SET, with the cache left at its `Agnostic` default. That
/// test encoded the bug — it passed precisely because nothing constrained the
/// cache.
#[test]
fn every_group_content_branch_pins_and_strictly_scopes_exactly_one_host() {
    let host = two_host_host();
    let query = host
        .group_contents_query(
            "room",
            nmp::Filter {
                kinds: Some(BTreeSet::from([9u16, 30315])),
                ..nmp::Filter::default()
            },
        )
        .expect("a plain selection scopes");

    assert_eq!(query.branches().len(), 2, "one complete branch per host");
    let h = IndexedTagName::new('h').unwrap();
    for (branch, expected) in query.branches().iter().zip([HOST_A, HOST_B]) {
        assert_eq!(branch.source, pinned_to(expected));
        assert_eq!(branch.cache, CacheMode::Strict);
        assert_eq!(branch.access, AccessContext::Public);
        assert_eq!(branch.selection.kinds, Some(BTreeSet::from([9, 30315])));
        assert_eq!(
            branch.selection.tags.get(&h),
            Some(&Binding::Literal(BTreeSet::from(["room".to_string()])))
        );
    }
}

#[test]
fn every_group_record_branch_pins_and_strictly_scopes_exactly_one_host() {
    let host = two_host_host();
    let query = host.group_records_query("room").expect("records scope");

    assert_eq!(query.branches().len(), 2);
    let d = IndexedTagName::new('d').unwrap();
    for (branch, expected) in query.branches().iter().zip([HOST_A, HOST_B]) {
        assert_eq!(branch.source, pinned_to(expected));
        assert_eq!(branch.cache, CacheMode::Strict);
        assert_eq!(
            branch.selection.kinds,
            Some(BTreeSet::from([39000u16, 39001, 39002])),
            "NMP's own NIP-29 discovery vocabulary owns which kinds describe a group"
        );
        assert_eq!(
            branch.selection.tags.get(&d),
            Some(&Binding::Literal(BTreeSet::from(["room".to_string()])))
        );
    }
}

#[test]
fn the_unpredicated_group_listing_is_also_per_host_and_strict() {
    let host = two_host_host();
    let query = host.all_group_metadata_query().expect("listing scope");

    assert_eq!(query.branches().len(), 2);
    for (branch, expected) in query.branches().iter().zip([HOST_A, HOST_B]) {
        assert_eq!(branch.source, pinned_to(expected));
        assert_eq!(branch.cache, CacheMode::Strict);
        assert_eq!(branch.selection.kinds, Some(BTreeSet::from([39000u16])));
    }
}

/// A group read REFUSES a caller-supplied `#h`: the retained group id is the
/// only source of that row. Mosaico can therefore no longer spell a group
/// scope as a raw tag by accident.
#[test]
fn a_caller_supplied_context_constraint_is_refused_not_overwritten() {
    let host = two_host_host();
    let mut filter = nmp::Filter {
        kinds: Some(BTreeSet::from([9u16])),
        ..nmp::Filter::default()
    };
    filter.tags.insert(
        IndexedTagName::new('h').unwrap(),
        Binding::Literal(BTreeSet::from(["elsewhere".to_string()])),
    );
    let error = host
        .group_contents_query("room", filter)
        .expect_err("a caller-supplied h constraint is refused");
    assert!(
        format!("{error}").contains("belongs to the group"),
        "{error}"
    );
}

/// Profiles are the one deliberate `Agnostic` read: kind:0 is
/// self-authenticating and the indexer is pinned precisely so it can answer
/// for relays outside the app's own set.
#[test]
fn profile_query_is_scoped_to_exact_authors_and_stays_provenance_agnostic() {
    let host = two_host_host();
    let author = "a".repeat(64);
    let live = host
        .live_query(
            &SubscriptionQuery::Profile {
                pubkey: author.clone(),
            },
            AccessContext::Public,
        )
        .unwrap();

    assert_eq!(live.branches().len(), 1);
    assert_eq!(
        live.branches()[0].selection.authors,
        Some(Binding::Literal(BTreeSet::from([author])))
    );
    assert_eq!(live.branches()[0].cache, CacheMode::Agnostic);
}

/// Every read pinned to the GROUP hosts is strict, including the ones NMP's
/// NIP-29 vocabulary does not mint.
#[test]
fn group_host_pinned_reads_are_strict_even_when_not_nip29_shaped() {
    let host = two_host_host();
    for query in [
        SubscriptionQuery::Kinds {
            kinds: BTreeSet::from([9000u16]),
        },
        SubscriptionQuery::Mentions {
            pubkey: "b".repeat(64),
            kinds: BTreeSet::from([9u16]),
        },
        SubscriptionQuery::References {
            event_id: "c".repeat(64),
            kinds: BTreeSet::from([30315u16]),
        },
    ] {
        let live = host.live_query(&query, AccessContext::Public).unwrap();
        assert_eq!(
            live.branches()[0].cache,
            CacheMode::Strict,
            "{query:?} must not inherit the Agnostic default"
        );
    }
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
