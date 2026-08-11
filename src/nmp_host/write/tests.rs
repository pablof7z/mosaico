use super::group_management::require_every_group_host_published;
use super::*;
use nmp::{ReceiptResult, RelayState, WriteOutcome};
use nostr::{EventBuilder, Kind, RelayUrl};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

mod group_door;

#[tokio::test]
async fn sign_event_serializes_distinct_accounts_and_restores_selection() {
    let host = Arc::new(NmpHost::open(&[], None, None, &Keys::generate()).unwrap());
    let a = Keys::generate();
    let b = Keys::generate();

    let (event_a, event_b) = tokio::join!(
        host.sign_event(EventBuilder::text_note("from a"), &a),
        host.sign_event(EventBuilder::text_note("from b"), &b),
    );
    let event_a = event_a.unwrap();
    let event_b = event_b.unwrap();

    assert_eq!(event_a.pubkey, a.public_key());
    assert_eq!(event_b.pubkey, b.public_key());
    assert!(event_a.verify().is_ok());
    assert!(event_b.verify().is_ok());
    assert_eq!(host.engine.active_account().unwrap(), None);
}

pub(super) fn one_host() -> NmpHost {
    NmpHost::open(
        &["wss://relay.example.com".into()],
        None,
        None,
        &Keys::generate(),
    )
    .unwrap()
}

/// The whole optimistic path rests on this: the id returned WITHOUT waiting is
/// the id NMP froze at acceptance, and Mosaico learned it by ASKING NMP rather
/// than by reimplementing NIP-01's hashing rule. A real engine is used
/// deliberately -- a scripted receipt stream would only prove Mosaico agrees
/// with itself.
#[tokio::test]
async fn the_returned_id_is_the_one_nmp_froze_and_nothing_derived_it() {
    let host = one_host();
    let keys = Keys::generate();
    let builder = EventBuilder::new(Kind::TextNote, "optimistic")
        .custom_created_at(nostr::Timestamp::from(1_700_000_000));

    let returned = host.publish_group("room-a", builder, &keys).unwrap();

    let entries = host
        .engine
        .publish_queue()
        .expect("the publish queue is readable");
    assert!(
        entries.iter().any(|entry| entry.event_id == returned),
        "NMP froze {:?}, none of which is the returned {returned}",
        entries.iter().map(|e| e.event_id).collect::<Vec<_>>()
    );
}

/// Each write's id rides its OWN acceptance answer. Two writes accepted back
/// to back must come back as two different ids, each naming its own durable
/// receipt -- which is what makes reading the id off the stream the caller
/// already holds correct, and not merely cheaper.
#[tokio::test]
async fn concurrent_writes_each_get_their_own_frozen_id() {
    let host = one_host();
    let keys = Keys::generate();

    let first = host
        .publish_group("room-a", EventBuilder::new(Kind::TextNote, "one"), &keys)
        .unwrap();
    let second = host
        .publish_group("room-a", EventBuilder::new(Kind::TextNote, "two"), &keys)
        .unwrap();

    assert_ne!(first, second);
    let entries = host.engine.publish_queue().unwrap();
    for id in [first, second] {
        assert!(
            entries.iter().any(|entry| entry.event_id == id),
            "{id} is not in the queue"
        );
    }
}

/// The durable half of write visibility: an accepted write is readable back
/// out of NMP's own queue, with no receipt id kept and no stream held open.
#[tokio::test]
async fn an_accepted_write_is_visible_in_the_queue_snapshot_without_any_bookkeeping() {
    let host = one_host();
    assert_eq!(host.publish_queue_snapshot().outstanding, 0);

    let keys = Keys::generate();
    let builder = EventBuilder::new(Kind::TextNote, "outstanding");
    host.publish_group("room-a", builder, &keys).unwrap();

    let snapshot = host.publish_queue_snapshot();
    assert!(snapshot.unreadable.is_none(), "{snapshot:?}");
    assert_eq!(snapshot.outstanding, 1);
    // A signer IS attached and the route is explicit, so nothing about this
    // write needs a person -- it is in flight, not stuck.
    assert!(snapshot.stuck.is_empty(), "{snapshot:?}");
}

/// `publish` returning `Ok` IS acceptance, and acceptance is not viability:
/// nothing here waits for a relay, so an offline host still returns promptly.
#[tokio::test]
async fn an_offline_relay_does_not_delay_acceptance() {
    let host = one_host();
    let keys = Keys::generate();
    let builder = EventBuilder::new(Kind::TextNote, "no spinner");

    let started = std::time::Instant::now();
    host.publish_group("room-a", builder, &keys)
        .expect("acceptance never depends on a relay");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_bounded_result_wait_does_not_turn_disconnection_into_an_unbounded_doctor() {
    let host = Arc::new(one_host());
    let started = std::time::Instant::now();
    let error = host
        .publish_group_result_within(
            "room-a",
            EventBuilder::new(Kind::TextNote, "bounded doctor"),
            &Keys::generate(),
            Duration::from_millis(20),
        )
        .await
        .expect_err("an unreachable relay cannot produce a terminal result immediately");

    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        format!("{error:#}").contains("remains in NMP's durable queue"),
        "{error:#}"
    );
    assert_eq!(host.publish_queue_snapshot().outstanding, 1);
}

/// A group write with no configured host cannot resolve, and says so at the
/// door rather than taking custody of something that can never go anywhere.
#[tokio::test]
async fn a_group_write_with_no_configured_host_is_refused_at_the_door() {
    let host = NmpHost::open(&[], None, None, &Keys::generate()).unwrap();
    let error = host
        .publish_group(
            "room-a",
            EventBuilder::new(Kind::TextNote, "nowhere"),
            &Keys::generate(),
        )
        .expect_err("a group needs a host");
    assert!(
        error
            .to_string()
            .contains("no configured NIP-29 group host"),
        "{error:#}"
    );
}

#[test]
fn terminal_result_requires_every_group_host_to_publish() {
    let accepted = RelayUrl::parse("wss://accepted.example").unwrap();
    let rejected = RelayUrl::parse("wss://rejected.example").unwrap();
    let success = ReceiptResult {
        outcome: WriteOutcome::Settled,
        relays: BTreeMap::from([(accepted.clone(), RelayState::Published)]),
    };
    require_every_group_host_published(&success).unwrap();

    let mixed = ReceiptResult {
        outcome: WriteOutcome::Settled,
        relays: BTreeMap::from([
            (accepted, RelayState::Published),
            (
                rejected,
                RelayState::Rejected {
                    reason: "not an administrator".into(),
                },
            ),
        ]),
    };
    let error = require_every_group_host_published(&mixed).unwrap_err();
    let rendered = format!("{error:#}");
    assert!(rendered.contains("wss://rejected.example"), "{rendered}");
    assert!(rendered.contains("not an administrator"), "{rendered}");
}

#[test]
fn local_or_whole_write_terminal_is_never_called_relay_success() {
    for result in [
        ReceiptResult {
            outcome: WriteOutcome::NoDestination,
            relays: BTreeMap::new(),
        },
        ReceiptResult {
            outcome: WriteOutcome::Settled,
            relays: BTreeMap::new(),
        },
    ] {
        assert!(require_every_group_host_published(&result).is_err());
    }
}
