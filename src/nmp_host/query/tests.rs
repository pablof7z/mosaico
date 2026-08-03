//! The one property `query.rs` exists to guarantee: both host-scoping axes
//! are stamped, per host, on every read Mosaico opens.

use std::collections::BTreeSet;

use nmp::{AccessContext, Binding, CacheMode, IndexedTagName, RelayUrl, SourceAuthority};
use nostr::Keys;

use crate::nmp_host::NmpHost;
use crate::reconcile::SubscriptionQuery;

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
