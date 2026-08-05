use super::*;
use nmp::{fifo_channel, RelayState, SigningState, WriteOutcome};

#[path = "tests/plumbing.rs"]
mod plumbing;

fn event_id(byte: u8) -> EventId {
    EventId::from_slice(&[byte; 32]).unwrap()
}

fn relay() -> nmp::RelayUrl {
    nmp::RelayUrl::parse("wss://relay.example.com").unwrap()
}

/// Parked on a signer that is not attached. Never terminal, and nothing
/// expires it — the shape used wherever a stream must stay open.
fn awaiting_signer() -> WriteFact {
    WriteFact::Signing(SigningState::AwaitingSigner {
        pubkey: nostr::Keys::generate().public_key(),
    })
}

fn signed(id: EventId) -> WriteFact {
    WriteFact::Signing(SigningState::Signed { event_id: id })
}

/// The signer answered and said no. Terminal for the whole write.
fn signer_refused(reason: &str) -> WriteFact {
    WriteFact::Signing(SigningState::Refused {
        reason: reason.into(),
    })
}

fn published() -> WriteFact {
    WriteFact::Relay {
        relay: relay(),
        state: RelayState::Published,
    }
}

fn persistence_stalled(detail: &str) -> WriteFact {
    WriteFact::Relay {
        relay: relay(),
        state: RelayState::Waiting(nmp::RelayWaiting::PersistenceStalled {
            detail: detail.into(),
        }),
    }
}

/// The whole-write terminal. Every receipt stream ends with exactly one of
/// these, which is why no path here has to infer an ending from silence.
fn settled() -> WriteFact {
    WriteFact::Outcome(WriteOutcome::Settled)
}

fn wait_for_failure(observer: &BackgroundReceiptObserver, source_ref: &str) {
    observer
        .evidence
        .wait_for_failure(source_ref, Duration::from_secs(1));
}

#[test]
fn signing_and_relay_facts_are_not_terminal_and_settlement_records_success() {
    let observer = BackgroundReceiptObserver::start_with(4, 2, Duration::from_secs(1)).unwrap();
    let id = event_id(1);
    let permit = observer.reserve("profile", id, 1).unwrap();
    let (sender, receiver) = fifo_channel();
    assert!(sender.send(awaiting_signer()));
    observer
        .observe(
            permit,
            "profile",
            id,
            vec![("0:wss://relay.example.com".into(), receiver)],
            true,
        )
        .unwrap();
    assert!(observer.snapshot().last_success.is_none());

    assert!(sender.send(signed(id)));
    // A relay ack closes ONE lane; the write is over when NMP says so.
    assert!(sender.send(published()));
    assert!(observer.snapshot().last_success.is_none());
    assert!(sender.send(settled()));
    observer.wait_idle();
    let snapshot = observer.snapshot();
    assert_eq!(
        snapshot.last_success.unwrap().status,
        BackgroundWriteTerminalStatus::Acked
    );
    assert!(snapshot.last_failure.is_none());
}

#[test]
fn exact_terminal_failure_is_correlated_after_acceptance() {
    let observer = BackgroundReceiptObserver::start_with(4, 2, Duration::from_secs(1)).unwrap();
    let id = event_id(2);
    let permit = observer.reserve("status", id, 1).unwrap();
    let (sender, receiver) = fifo_channel();
    let detail = "durable-store persistence failure [fault=latched durability=absent reopen=required]: Previous I/O error occurred";
    assert!(sender.send(awaiting_signer()));
    assert!(sender.send(signer_refused(detail)));
    observer
        .observe(
            permit,
            "status",
            id,
            vec![("0:wss://relay.example.com".into(), receiver)],
            true,
        )
        .unwrap();
    observer.wait_idle();

    let evidence = observer.snapshot().last_failure.unwrap();
    assert_eq!(evidence.operation, "status");
    assert_eq!(evidence.source_ref, id.to_hex());
    assert_eq!(evidence.target, "0:wss://relay.example.com");
    assert_eq!(
        evidence.status,
        BackgroundWriteTerminalStatus::SignerRefused
    );
    assert_eq!(evidence.detail, detail);
    assert!(evidence.observed_at > 0);
    assert!(!evidence.detail.contains("member"));
    assert!(!evidence.detail.contains("admin"));
}

#[test]
fn persistence_blockage_remains_visible_after_later_ack() {
    let observer = BackgroundReceiptObserver::start_with(2, 1, Duration::from_secs(1)).unwrap();
    let id = event_id(10);
    let permit = observer.reserve("profile", id, 1).unwrap();
    let (sender, receiver) = fifo_channel();
    assert!(sender.send(persistence_stalled("attempt log stall")));
    assert!(sender.send(published()));
    assert!(sender.send(settled()));
    observer
        .observe(
            permit,
            "profile",
            id,
            vec![("0:wss://relay.example.com".into(), receiver)],
            true,
        )
        .unwrap();
    observer.wait_idle();

    let snapshot = observer.snapshot();
    assert_eq!(
        snapshot.last_failure.unwrap().status,
        BackgroundWriteTerminalStatus::PersistenceStalled
    );
    assert_eq!(
        snapshot.last_success.unwrap().status,
        BackgroundWriteTerminalStatus::Acked
    );
}

/// A newer write winning the same replaceable coordinate is the steady state
/// of presence renewal, not a fault. It must produce no failure evidence — and
/// it must not be reported as an acknowledgement either, because nothing
/// reached a relay.
#[test]
fn a_superseded_write_is_neither_a_failure_nor_an_acknowledgement() {
    let observer = BackgroundReceiptObserver::start_with(2, 1, Duration::from_secs(1)).unwrap();
    let id = event_id(15);
    let permit = observer.reserve("status", id, 1).unwrap();
    let (sender, receiver) = fifo_channel();
    assert!(sender.send(signed(id)));
    assert!(sender.send(WriteFact::Outcome(WriteOutcome::NotSent(
        nmp::NotSentReason::Superseded
    ))));
    observer
        .observe(
            permit,
            "status",
            id,
            vec![("0:superseded".into(), receiver)],
            true,
        )
        .unwrap();
    observer.wait_idle();

    let snapshot = observer.snapshot();
    assert!(snapshot.last_failure.is_none());
    assert!(snapshot.last_gap.is_none());
    assert!(snapshot.last_success.is_none());
}

#[test]
fn silent_first_stream_does_not_hide_later_failure_for_the_same_event() {
    let observer = BackgroundReceiptObserver::start_with(4, 2, Duration::from_secs(2)).unwrap();
    let id = event_id(3);
    let permit = observer.reserve("profile", id, 2).unwrap();
    let (_held_sender, held_receiver) = fifo_channel();
    let (failed_sender, failed_receiver) = fifo_channel();
    assert!(failed_sender.send(signer_refused("second write failed")));
    let started = Instant::now();
    observer
        .observe(
            permit,
            "profile",
            id,
            vec![
                ("0:wss://silent.example.com".into(), held_receiver),
                ("1:wss://failed.example.com".into(), failed_receiver),
            ],
            true,
        )
        .unwrap();
    wait_for_failure(&observer, &id.to_hex());

    assert!(started.elapsed() < Duration::from_secs(1));
    let failure = observer.snapshot().last_failure.unwrap();
    assert_eq!(failure.source_ref, id.to_hex());
    assert_eq!(failure.target, "1:wss://failed.example.com");
}

#[test]
fn four_silent_streams_do_not_hide_immediate_fifth_stream_failure() {
    let observer = BackgroundReceiptObserver::start_with(5, 4, Duration::from_secs(2)).unwrap();
    let id = event_id(11);
    let permit = observer.reserve("status", id, 5).unwrap();
    let mut held_senders = Vec::new();
    let mut receivers = Vec::new();
    for index in 0..4 {
        let (sender, receiver) = fifo_channel();
        held_senders.push(sender);
        receivers.push((format!("{index}:silent"), receiver));
    }
    let (failed_sender, failed_receiver) = fifo_channel();
    assert!(failed_sender.send(signer_refused("fifth stream failed immediately")));
    receivers.push(("4:failed".into(), failed_receiver));

    let started = Instant::now();
    observer
        .observe(permit, "status", id, receivers, true)
        .unwrap();
    wait_for_failure(&observer, &id.to_hex());

    assert!(started.elapsed() < Duration::from_secs(1));
    let failure = observer.snapshot().last_failure.unwrap();
    assert_eq!(failure.target, "4:failed");
    assert_eq!(failure.detail, "fifth stream failed immediately");
    drop(held_senders);
}
