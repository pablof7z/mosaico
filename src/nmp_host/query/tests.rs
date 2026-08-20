//! The one property `query.rs` exists to guarantee: both host-scoping axes
//! are stamped, per host, on every read Mosaico opens.

use std::collections::BTreeSet;

use nmp::{Binding, CacheMode, IndexedTagName, ReadRouting, RelayUrl};
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

fn pinned_to(url: &str) -> ReadRouting {
    ReadRouting::Explicit(vec![RelayUrl::parse(url).unwrap()])
}

/// mosaico#741. The two host-scoping axes are stamped, per host, on every
/// NIP-29 read Mosaico opens: `ReadRouting::Explicit` for which relays are
/// ASKED and `CacheMode::Strict` for which cached rows may ANSWER. A NIP-29
/// read needs both pointed at the same host, because 39000/39001/39002 are
/// relay-SIGNED and two relays hosting one group id are two independent groups.
#[test]
fn every_group_record_branch_pins_and_strictly_scopes_exactly_one_host() {
    let host = two_host_host();
    let query = host.group_records_query("room").expect("records scope");

    assert_eq!(query.branches().len(), 2);
    let d = IndexedTagName::new('d').unwrap();
    for (branch, expected) in query.branches().iter().zip([HOST_A, HOST_B]) {
        assert_eq!(branch.routing, pinned_to(expected));
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
            None,
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
    let query = SubscriptionQuery::References {
        event_id: "c".repeat(64),
        kinds: BTreeSet::from([30315u16]),
    };
    let live = host.live_query(&query, None).unwrap();
    assert_eq!(
        live.branches()[0].cache,
        CacheMode::Strict,
        "{query:?} must not inherit the Agnostic default"
    );
}

#[test]
fn broad_group_contents_is_one_branch_with_multivalue_h_over_all_relays() {
    let host = two_host_host();
    let query = host
        .live_query(
            &SubscriptionQuery::GroupContentsMulti {
                groups: BTreeSet::from(["a".to_string(), "b".to_string()]),
                kinds: BTreeSet::from([9u16, 7, 30315, 9000]),
            },
            None,
        )
        .expect("broad #h scopes");

    // One observation (one demand) over all group relays — not per-host branches.
    // Self-authenticating content (9/7/30315/9000) needs no per-host branching.
    assert_eq!(query.branches().len(), 1);
    let branch = &query.branches()[0];
    assert_eq!(
        branch.routing,
        ReadRouting::Explicit(vec![
            RelayUrl::parse(HOST_A).unwrap(),
            RelayUrl::parse(HOST_B).unwrap()
        ]),
        "pinned to all group relays in canonical order"
    );
    assert_eq!(branch.cache, CacheMode::Strict);
    assert_eq!(branch.authenticate_as, None);
    assert_eq!(
        branch.selection.kinds,
        Some(BTreeSet::from([9u16, 7, 30315, 9000]))
    );
    let h = IndexedTagName::new('h').unwrap();
    assert_eq!(
        branch.selection.tags.get(&h),
        Some(&Binding::Literal(BTreeSet::from([
            "a".to_string(),
            "b".to_string()
        ]))),
        "multi-value #h covers all known channels"
    );
}
