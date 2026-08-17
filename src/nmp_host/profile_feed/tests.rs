//! Profile feed tests (mosaico#837, slice 1).
//!
//! Query shape, the drain's upsert/remove/latest-wins logic, and real end-to-end
//! delivery through an in-process Nostr relay + the NMP engine.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nostr::Keys;

use crate::domain::{AgentRef, DomainEvent, Profile as DomainProfile};
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::nmp_host::{NmpHost, PlainRelay, ProfileFeed};

const HOST_A: &str = "wss://a.example.com";
const HOST_B: &str = "wss://b.example.com";

/// One non-backend kind:0 profile event signed by `keys`, carrying `name` as
/// its metadata name and `agent_slug` as its `["agent-slug"]` tag.
fn kind0_event(
    keys: &Keys,
    name: &str,
    agent_slug: &str,
    host: &str,
    created_at: u64,
) -> nostr::Event {
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
    Nip29WireCodec
        .encode_event(&DomainEvent::Profile(profile))
        .expect("encode kind:0")
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(keys)
        .expect("sign kind:0")
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

#[test]
fn profile_feed_query_pins_exact_author_set_and_stays_agnostic() {
    let host = NmpHost::open(
        &[HOST_A.to_string(), HOST_B.to_string()],
        None,
        None,
        &Keys::generate(),
    )
    .expect("open host over two profile relays");
    let author = "a".repeat(64);
    let live = host
        .profile_feed_query(&BTreeSet::from([author.clone()]))
        .expect("build single-member feed query");

    assert_eq!(
        live.branches().len(),
        1,
        "one branch over the profile relays"
    );
    let branch = &live.branches()[0];
    assert_eq!(branch.cache, nmp::CacheMode::Agnostic);
    assert_eq!(branch.access, nmp::AccessContext::Public);
    assert_eq!(branch.selection.kinds, Some(BTreeSet::from([0u16])));
    assert_eq!(
        branch.selection.authors,
        Some(nmp::Binding::Literal(BTreeSet::from([author])))
    );
}

#[test]
fn profile_feed_query_scopes_the_exact_multi_member_author_set() {
    let host = NmpHost::open(&[HOST_A.to_string()], None, None, &Keys::generate())
        .expect("open host over one profile relay");
    let authors = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
    let live = host
        .profile_feed_query(&authors.iter().cloned().collect())
        .expect("build multi-member feed query");

    assert_eq!(live.branches().len(), 1);
    assert_eq!(live.branches()[0].cache, nmp::CacheMode::Agnostic);
    assert_eq!(
        live.branches()[0].selection.kinds,
        Some(BTreeSet::from([0u16]))
    );
    let expected: BTreeSet<String> = authors.iter().cloned().collect();
    assert_eq!(
        live.branches()[0].selection.authors,
        Some(nmp::Binding::Literal(expected))
    );
}

#[test]
fn apply_delta_upserts_latest_and_removes_by_event_id() {
    let feed = ProfileFeed::default();
    let keys = Keys::generate();
    let pk = keys.public_key().to_hex();

    let v1 = kind0_event(&keys, "v1-handle", "", "laptop", 100);
    let v2 = kind0_event(&keys, "v2-handle", "", "laptop", 200);

    feed.apply_delta(nmp::RowDelta::Added(nmp::Row {
        event: v1.clone(),
        sources: BTreeSet::new(),
    }));
    assert_eq!(
        feed.profile(&pk).map(|p| p.name).as_deref(),
        Some("v1-handle")
    );

    // NMP delivers a newer replaceable kind:0 as Added(new); the feed's upsert
    // keeps the latest delivery per pubkey.
    feed.apply_delta(nmp::RowDelta::Added(nmp::Row {
        event: v2.clone(),
        sources: BTreeSet::new(),
    }));
    assert_eq!(
        feed.profile(&pk).map(|p| p.name).as_deref(),
        Some("v2-handle")
    );

    // Removing the stored event id drops the profile; a stale removal of the
    // superseded id is a no-op.
    feed.apply_delta(nmp::RowDelta::Removed(v1.id));
    assert_eq!(
        feed.profile(&pk).map(|p| p.name).as_deref(),
        Some("v2-handle"),
        "removing the superseded id must not drop the current profile"
    );
    feed.apply_delta(nmp::RowDelta::Removed(v2.id));
    assert!(feed.profile(&pk).is_none());
}

#[test]
fn feed_drains_real_kind0_and_resolves_latest_then_live_updates() {
    let a = Keys::generate();
    let b = Keys::generate();
    let pk_a = a.public_key().to_hex();
    let pk_b = b.public_key().to_hex();

    // Seed an older and a newer kind:0 for A (NMP keeps the newer per author)
    // plus one for B.
    let relay = PlainRelay::spawn([
        kind0_event(&a, "a-older", "", "laptop", 100),
        kind0_event(&a, "a-newer", "", "laptop", 200),
        kind0_event(&b, "b-handle", "", "tower", 150),
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
    relay.inject(kind0_event(&a, "a-live", "", "laptop", 300));
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

#[test]
fn set_members_replaces_the_observed_author_set() {
    let a = Keys::generate();
    let b = Keys::generate();
    let pk_a = a.public_key().to_hex();
    let pk_b = b.public_key().to_hex();

    let relay = PlainRelay::spawn([
        kind0_event(&a, "a-handle", "", "laptop", 100),
        kind0_event(&b, "b-handle", "", "tower", 150),
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
