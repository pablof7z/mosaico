use super::*;
use nmp::{ReceiptId, RefuseReason, RelayState, WriteOutcome};
use nostr::{EventId, Keys, PublicKey, Timestamp};
use std::collections::BTreeSet;

fn entry(id: u8) -> PublishQueueEntry {
    PublishQueueEntry {
        receipt_id: ReceiptId(u64::from(id)),
        event_id: EventId::from_slice(&[id; 32]).unwrap(),
        pubkey: Keys::generate().public_key(),
        accepted_at: Timestamp::from(1_700_000_000),
        signing: SigningState::Signed {
            event_id: EventId::from_slice(&[id; 32]).unwrap(),
        },
        relays: BTreeSet::new(),
        route_complete: true,
        relay_states: Vec::new(),
        outcome: None,
        persistence_fault: None,
    }
}

fn awaiting(id: u8, pubkey: PublicKey) -> PublishQueueEntry {
    PublishQueueEntry {
        signing: SigningState::AwaitingSigner { pubkey },
        ..entry(id)
    }
}

fn in_flight(id: u8, pubkey: PublicKey) -> PublishQueueEntry {
    PublishQueueEntry {
        signing: SigningState::InFlight { pubkey },
        ..entry(id)
    }
}

#[test]
fn an_ordinary_in_flight_write_is_outstanding_and_not_stuck() {
    let snapshot = summarize(&[entry(1)]);
    assert_eq!(snapshot.entries, 1);
    assert_eq!(snapshot.outstanding, 1);
    assert!(snapshot.stuck.is_empty());
}

/// A write still learning where it goes parks deliberately and forever.
/// Reporting it as stuck would be Mosaico guessing at exactly the thing NMP
/// refuses to guess at.
#[test]
fn a_write_still_resolving_its_route_is_not_reported_as_stuck() {
    let unresolved = PublishQueueEntry {
        route_complete: false,
        ..entry(2)
    };
    assert!(summarize(&[unresolved]).stuck.is_empty());
}

/// A write whose signature is merely in flight must not be reported as stuck.
/// This is the shape of every healthy write for the moment between acceptance
/// and signature promotion.
#[test]
fn a_signature_in_flight_is_outstanding_and_never_reported_as_stuck() {
    let pubkey = Keys::generate().public_key();
    let snapshot = summarize(&[in_flight(3, pubkey)]);
    assert_eq!(snapshot.outstanding, 1);
    assert!(snapshot.stuck.is_empty(), "{snapshot:?}");
}

/// The falsifier for adopting NMP #1270. Before it, every unsigned write
/// projected as `AwaitingSigner`, so the genuinely parked write -- no signer
/// answers for this key, and no clock will ever end that -- could not be named
/// without also alarming on the healthy one above. It can now, so it is.
#[test]
fn a_write_no_signer_answers_for_is_named_as_stuck() {
    let pubkey = Keys::generate().public_key();
    let snapshot = summarize(&[awaiting(8, pubkey)]);
    assert_eq!(snapshot.outstanding, 1);
    assert_eq!(snapshot.stuck.len(), 1, "{snapshot:?}");
    assert!(
        snapshot.stuck[0].reason.contains(&pubkey.to_string()),
        "the parked key must be named so a person knows which signer to attach: {snapshot:?}"
    );
}

#[test]
fn a_signer_that_answered_no_is_stuck_with_its_exact_reason() {
    let refused = PublishQueueEntry {
        signing: SigningState::Refused {
            reason: "device is locked".into(),
        },
        ..entry(7)
    };
    let snapshot = summarize(&[refused]);
    assert_eq!(snapshot.stuck.len(), 1);
    assert!(snapshot.stuck[0].reason.contains("device is locked"));
}

/// Custody is not viability. A permanently-failed entry has an outcome, so it
/// is not outstanding -- and it still needs a person, so it is stuck.
#[test]
fn a_permanently_failed_entry_is_stuck_without_being_outstanding() {
    let refused = PublishQueueEntry {
        outcome: Some(WriteOutcome::Refused(RefuseReason::Tombstoned)),
        ..entry(4)
    };
    let snapshot = summarize(&[refused]);
    assert_eq!(snapshot.outstanding, 0);
    assert_eq!(snapshot.stuck.len(), 1);
    assert!(snapshot.stuck[0].reason.contains("Tombstoned"));
}

/// The latch outlives the ack, here as well as in the receipt evidence: an
/// operator must not lose the only signal that the disk is failing because a
/// relay accepted the event afterwards.
#[test]
fn a_latched_persistence_fault_survives_a_settled_outcome() {
    let settled_but_stalled = PublishQueueEntry {
        outcome: Some(WriteOutcome::Settled),
        persistence_fault: Some("attempt log stall".into()),
        relay_states: vec![(
            nmp::RelayUrl::parse("wss://relay.example.com").unwrap(),
            RelayState::Published,
        )],
        ..entry(5)
    };
    let snapshot = summarize(&[settled_but_stalled]);
    assert_eq!(snapshot.stuck.len(), 1);
    assert!(snapshot.stuck[0].reason.contains("attempt log stall"));
}

#[test]
fn a_settled_write_is_neither_outstanding_nor_stuck() {
    let settled = PublishQueueEntry {
        outcome: Some(WriteOutcome::Settled),
        ..entry(6)
    };
    let snapshot = summarize(&[settled]);
    assert_eq!(snapshot.outstanding, 0);
    assert!(snapshot.stuck.is_empty());
}

#[test]
fn the_named_list_is_bounded_and_says_how_many_it_left_out() {
    let entries: Vec<_> = (0..20u8)
        .map(|id| PublishQueueEntry {
            outcome: Some(WriteOutcome::Refused(RefuseReason::Tombstoned)),
            ..entry(id)
        })
        .collect();
    let snapshot = summarize(&entries);
    assert_eq!(snapshot.stuck.len(), NAMED_STUCK_WRITES);
    assert_eq!(snapshot.stuck_total, 20);
}
