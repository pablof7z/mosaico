//! What one NMP fact means to this observer.
//!
//! There is no classifier under test here because there is no classifier: NMP
//! separates whole-write facts from per-relay facts and ends every stream with
//! exactly one `WriteOutcome`. What remains Mosaico's is the naming, and these
//! pin the names that an operator acts on.

use super::*;
use nmp::{AuthDenialSource, NotSentReason, RelayState, RelayWaiting, SigningState, WriteOutcome};

fn relay() -> nmp::RelayUrl {
    nmp::RelayUrl::parse("wss://relay.example.com").unwrap()
}

fn fault(state: RelayState) -> Option<(BackgroundWriteTerminalStatus, String)> {
    LaneFacts::default().observe_relay(&relay(), state)
}

#[test]
fn an_unattached_signer_and_a_signature_are_both_silent() {
    assert!(facts::signer_refusal(SigningState::AwaitingSigner {
        pubkey: nostr::Keys::generate().public_key(),
    })
    .is_none());
    assert!(facts::signer_refusal(SigningState::Signed {
        event_id: nostr::EventId::from_slice(&[7; 32]).unwrap(),
    })
    .is_none());
}

#[test]
fn a_refused_signature_keeps_its_exact_reason() {
    let reason = "fault=latched: Previous I/O error occurred";
    assert_eq!(
        facts::signer_refusal(SigningState::Refused {
            reason: reason.into(),
        }),
        Some(reason.to_string())
    );
}

#[test]
fn ordinary_lane_states_are_not_faults() {
    assert!(fault(RelayState::Waiting(RelayWaiting::NotConnected)).is_none());
    assert!(fault(RelayState::Waiting(RelayWaiting::NeedsAuth)).is_none());
    assert!(fault(RelayState::Waiting(RelayWaiting::BackingOff {
        attempt: 3,
        eligible_at: nostr::Timestamp::from(7),
        cause: nmp::RetryCause::RelayRateLimited,
        detail: Some("slow down".into()),
    }))
    .is_none());
    assert!(fault(RelayState::Published).is_none());
}

/// The distinction mosaico#745 lost. A relay that authenticated the identity
/// and then refused the event is a different repair from the app's own policy
/// declining to authenticate — so the two must never share a status.
#[test]
fn an_auth_failure_is_never_reported_as_a_relay_rejecting_the_event() {
    let pubkey = nostr::Keys::generate().public_key();
    let (status, detail) = fault(RelayState::AuthFailed {
        pubkey,
        source: AuthDenialSource::Policy,
        reason: "this app does not authenticate here".into(),
    })
    .expect("an AUTH failure is a fault");
    assert_eq!(status, BackgroundWriteTerminalStatus::AuthFailed);
    assert_ne!(status, BackgroundWriteTerminalStatus::Rejected);
    assert!(detail.contains("Policy"), "{detail}");
    assert!(detail.contains(&pubkey.to_string()), "{detail}");
    assert!(detail.contains("does not authenticate here"), "{detail}");

    let (status, detail) = fault(RelayState::Rejected {
        reason: "blocked: pow too low".into(),
    })
    .expect("a rejection is a fault");
    assert_eq!(status, BackgroundWriteTerminalStatus::Rejected);
    assert!(detail.contains("pow too low"), "{detail}");
}

#[test]
fn a_persistence_stall_is_a_fault_the_moment_it_is_observed() {
    let (status, detail) = fault(RelayState::Waiting(RelayWaiting::PersistenceStalled {
        detail: "route revision stall".into(),
    }))
    .expect("a local persistence stall is a fault");
    assert_eq!(status, BackgroundWriteTerminalStatus::PersistenceStalled);
    assert!(detail.contains("route revision stall"), "{detail}");
}

#[test]
fn the_attempt_ceiling_is_named_as_such() {
    let (status, detail) = fault(RelayState::GaveUp).expect("a give-up is a fault");
    assert_eq!(status, BackgroundWriteTerminalStatus::GaveUp);
    assert!(detail.contains("ceiling"), "{detail}");
}

/// One relay given up on and another published is a success with a footnote.
/// The footnote was already filed when it was observed; the verdict is a
/// success.
#[test]
fn a_write_that_reached_a_relay_settles_as_acknowledged_despite_a_lost_lane() {
    let mut lanes = LaneFacts::default();
    assert!(lanes.observe_relay(&relay(), RelayState::GaveUp).is_some());
    assert!(lanes
        .observe_relay(&relay(), RelayState::Published)
        .is_none());
    assert!(matches!(
        lanes.settle(WriteOutcome::Settled),
        StreamOutcome::Acked
    ));
}

#[test]
fn a_write_that_reached_no_relay_settles_with_the_reason_it_did_not() {
    let mut lanes = LaneFacts::default();
    lanes.observe_relay(
        &relay(),
        RelayState::Rejected {
            reason: "invalid: missing h tag".into(),
        },
    );
    match lanes.settle(WriteOutcome::Settled) {
        StreamOutcome::Failure(BackgroundWriteTerminalStatus::Rejected, detail) => {
            assert!(detail.contains("missing h tag"), "{detail}");
        }
        _ => panic!("a settled write that reached no relay must name why"),
    }
}

#[test]
fn cancellation_is_terminal_and_explicit() {
    match LaneFacts::default().settle(WriteOutcome::NotSent(NotSentReason::Cancelled)) {
        StreamOutcome::Failure(BackgroundWriteTerminalStatus::Cancelled, detail) => {
            assert_eq!(detail, "write was cancelled before signature promotion");
        }
        _ => panic!("a cancelled write must be terminal"),
    }
}

#[test]
fn supersession_is_terminal_without_being_a_fault() {
    assert!(matches!(
        LaneFacts::default().settle(WriteOutcome::NotSent(NotSentReason::Superseded)),
        StreamOutcome::Superseded
    ));
}

#[test]
fn nowhere_to_publish_is_terminal_and_named() {
    match LaneFacts::default().settle(WriteOutcome::NoDestination) {
        StreamOutcome::Failure(BackgroundWriteTerminalStatus::NoDestination, detail) => {
            assert!(detail.contains("no relays"), "{detail}");
        }
        _ => panic!("routing that named nobody must be terminal"),
    }
}

/// A stale replaceable base keeps BOTH ids, because that is what makes the
/// failure recoverable without troubling anyone.
#[test]
fn a_stale_replaceable_base_keeps_the_ids_that_make_it_recoverable() {
    let expected = nostr::EventId::from_slice(&[1; 32]).unwrap();
    let actual = nostr::EventId::from_slice(&[2; 32]).unwrap();
    match LaneFacts::default().settle(WriteOutcome::Refused(
        nmp::RefuseReason::ReplaceableBaseChanged {
            expected: Some(expected),
            actual: Some(actual),
        },
    )) {
        StreamOutcome::Failure(BackgroundWriteTerminalStatus::Refused, detail) => {
            assert!(detail.contains(&expected.to_string()), "{detail}");
            assert!(detail.contains(&actual.to_string()), "{detail}");
        }
        _ => panic!("a refused acceptance must be terminal"),
    }
}
