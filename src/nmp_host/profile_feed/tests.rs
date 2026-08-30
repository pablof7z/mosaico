//! Profile feed tests (mosaico#837, slice 1).
//!
//! End-to-end delivery through an in-process Nostr relay and the fava engine.
//! Query-shape tests (branches/CacheMode/Binding) have been removed — those
//! checked NMP-internal structure that has no fava equivalent.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nostr::key::Keys;

use crate::domain::{AgentRef, DomainEvent, Profile as DomainProfile};
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::nmp_host::{NmpHost, PlainRelay, ProfileFeed};

/// One non-backend kind:0 profile event signed by `keys`, carrying `name` as
/// its metadata name and `agent_slug` as its `["agent-slug"]` tag.
async fn kind0_event(
    keys: &Keys,
    name: &str,
    agent_slug: &str,
    host: &str,
    created_at: u64,
) -> nostr::event::Event {
    let profile = DomainProfile {
        agent: AgentRef::new(keys.public_key().to_hex(), name),
        agent_slug: agent_slug.to_string(),
        host: host.to_string(),
        workspace: "ws".to_string(),
        owners: Vec::new(),
        is_backend: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
    };
    let builder = Nip29WireCodec
        .encode_event(&DomainEvent::Profile(profile))
        .expect("encode kind:0")
        .custom_created_at(nostr::types::Timestamp::from(created_at));
    let unsigned = fava::AuthoredEventBuilder::from_parts(
        keys.public_key(),
        builder.kind,
        builder.created_at.expect("kind:0 timestamp"),
        builder.tags.to_vec(),
        builder.content,
    )
    .build()
    .expect("build unsigned kind:0");
    let host = NmpHost::open(&[], None, None, keys).expect("open Fava signer");
    let signed = host.fava.sign(unsigned).await.expect("Fava signs kind:0");
    host.shutdown();
    signed
}

/// Poll the feed's synchronous read until it returns `want` or `deadline`.
fn wait_profile(feed: &ProfileFeed, pubkey: &str, want: Option<&str>, deadline: Instant) -> bool {
    while Instant::now() < deadline {
        match (feed.profile(pubkey), want) {
            (Some(p), Some(name)) if p.name == name => return true,
            (None, None) => return true,
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    feed.profile(pubkey)
        .map(|p| p.name)
        .as_deref()
        .zip(want)
        .is_some_and(|(got, want)| got == want)
        || (feed.profile(pubkey).is_none() && want.is_none())
}

#[tokio::test(flavor = "multi_thread")]
async fn feed_drains_real_kind0_and_resolves_latest_then_live_updates() {
    let a = Keys::generate();
    let b = Keys::generate();
    let pk_a = a.public_key().to_hex();
    let pk_b = b.public_key().to_hex();

    // Seed an older and a newer kind:0 for A (feed keeps the newer per author)
    // plus one for B.
    let relay = PlainRelay::spawn([
        kind0_event(&a, "a-older", "", "laptop", 100).await,
        kind0_event(&a, "a-newer", "", "laptop", 200).await,
        kind0_event(&b, "b-handle", "", "tower", 150).await,
    ]);
    let host = Arc::new(
        NmpHost::open(&[relay.url()], None, None, &Keys::generate())
            .expect("open host over the plain relay"),
    );
    let feed = Arc::new(ProfileFeed::new(host.clone()));
    feed.set_members(BTreeSet::from([pk_a.clone()]));

    let deadline = Instant::now() + Duration::from_secs(10);
    assert!(
        wait_profile(&feed, &pk_a, Some("a-newer"), deadline),
        "feed should receive the latest kind:0 for the member through real delivery"
    );
    assert!(
        feed.profile(&pk_b).is_none(),
        "B is not a member; its profile must not be observed"
    );

    // Live injection of an even newer kind:0 for A reaches the open drain.
    relay.inject(kind0_event(&a, "a-live", "", "laptop", 300).await);
    assert!(
        wait_profile(
            &feed,
            &pk_a,
            Some("a-live"),
            Instant::now() + Duration::from_secs(10)
        ),
        "feed should apply the live newer kind:0"
    );

    drop(feed);
    host.shutdown();
    relay.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_members_replaces_the_observed_author_set() {
    let a = Keys::generate();
    let b = Keys::generate();
    let pk_a = a.public_key().to_hex();
    let pk_b = b.public_key().to_hex();

    let relay = PlainRelay::spawn([
        kind0_event(&a, "a-handle", "", "laptop", 100).await,
        kind0_event(&b, "b-handle", "", "tower", 150).await,
    ]);
    let host = Arc::new(
        NmpHost::open(&[relay.url()], None, None, &Keys::generate())
            .expect("open host over the plain relay"),
    );
    let feed = Arc::new(ProfileFeed::new(host.clone()));
    feed.set_members(BTreeSet::from([pk_a.clone()]));
    assert!(
        wait_profile(
            &feed,
            &pk_a,
            Some("a-handle"),
            Instant::now() + Duration::from_secs(10)
        ),
        "A's profile is picked up for the initial member set"
    );

    // Swapping to {B} drops A and opens a new observation that delivers B.
    feed.set_members(BTreeSet::from([pk_b.clone()]));
    let deadline = Instant::now() + Duration::from_secs(10);
    assert!(
        wait_profile(&feed, &pk_a, None, deadline),
        "A's profile is dropped once A leaves the member set"
    );
    assert!(
        wait_profile(
            &feed,
            &pk_b,
            Some("b-handle"),
            Instant::now() + Duration::from_secs(10)
        ),
        "B's profile is picked up once B joins the member set"
    );

    drop(feed);
    host.shutdown();
    relay.shutdown();
}
