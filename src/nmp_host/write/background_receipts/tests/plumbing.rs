//! The observer's own plumbing: admission, capacity, worker liveness, and the
//! bounded observation window. None of these are facts about a write — they are
//! facts about whether Mosaico was still watching, which is why every one of
//! them is filed as a gap.

use super::super::*;
use nmp::fifo_channel;

use super::{awaiting_signer, event_id};

#[test]
fn worker_panic_records_distinct_gap_and_releases_capacity() {
    let observer = BackgroundReceiptObserver::start_with(1, 1, Duration::from_secs(1)).unwrap();
    let id = event_id(12);
    let permit = observer.reserve("profile", id, 1).unwrap();
    let (_sender, receiver) = fifo_channel();
    observer
        .observe(
            permit,
            "profile",
            id,
            vec![("panic:test-worker".into(), receiver)],
            true,
        )
        .unwrap();
    observer.wait_idle();

    let snapshot = observer.snapshot();
    assert_eq!(snapshot.pending, 0);
    assert_eq!(
        snapshot.last_gap.unwrap().status,
        BackgroundWriteGapStatus::WorkerLost
    );

    let permit = observer
        .reserve("status", event_id(13), 1)
        .expect("worker panic must release its RAII stream capacity");
    drop(permit);
}

#[test]
fn saturation_and_closed_admission_are_observer_gaps() {
    let observer = BackgroundReceiptObserver::start_with(1, 1, Duration::from_secs(1)).unwrap();
    let permit = observer.reserve("status", event_id(5), 1).unwrap();
    let error = observer
        .reserve("profile", event_id(6), 1)
        .err()
        .expect("saturated admission must fail");
    assert!(error.to_string().contains("capacity"));
    assert_eq!(
        observer.snapshot().last_gap.unwrap().status,
        BackgroundWriteGapStatus::CapacityFull
    );
    drop(permit);

    observer.begin_shutdown();
    let error = observer
        .reserve("status", event_id(7), 1)
        .err()
        .expect("closed admission must fail");
    assert!(error.to_string().contains("closed"));
    assert_eq!(
        observer.snapshot().last_gap.unwrap().status,
        BackgroundWriteGapStatus::ObserverClosed
    );
}

/// The observation window is process-local and bounded; the write it was
/// watching is not. NMP keeps the obligation in its publish queue, so ending
/// this observation is a hole in what we saw, never a verdict on the write.
#[test]
fn receipt_timeout_is_a_gap_not_a_write_failure() {
    let observer = BackgroundReceiptObserver::start_with(2, 1, Duration::from_millis(30)).unwrap();
    let id = event_id(8);
    let permit = observer.reserve("status", id, 1).unwrap();
    let (_sender, receiver) = fifo_channel();
    observer
        .observe(
            permit,
            "status",
            id,
            vec![("0:held".into(), receiver)],
            true,
        )
        .unwrap();
    observer.wait_idle();

    let snapshot = observer.snapshot();
    assert!(snapshot.last_failure.is_none());
    assert_eq!(
        snapshot.last_gap.unwrap().status,
        BackgroundWriteGapStatus::ReceiptTimeout
    );
}

#[test]
fn lagged_receipt_is_an_explicit_observation_gap() {
    let observer = BackgroundReceiptObserver::start_with(1, 1, Duration::from_secs(1)).unwrap();
    let id = event_id(14);
    let permit = observer.reserve("status", id, 1).unwrap();
    let (sender, receiver) = fifo_channel();
    for _ in 0..nmp::FACT_CHANNEL_CAPACITY {
        assert!(sender.send(awaiting_signer()));
    }
    assert!(!sender.send(awaiting_signer()));
    observer
        .observe(
            permit,
            "status",
            id,
            vec![("0:lagged".into(), receiver)],
            true,
        )
        .unwrap();
    observer.wait_idle();

    let snapshot = observer.snapshot();
    assert!(snapshot.last_failure.is_none());
    assert_eq!(
        snapshot.last_gap.unwrap().status,
        BackgroundWriteGapStatus::ReceiptLagged
    );
}

#[test]
fn shutdown_wakes_128_held_receipts_and_joins_under_one_second() {
    let observer = BackgroundReceiptObserver::start_with(128, 4, Duration::from_secs(30)).unwrap();
    let id = event_id(9);
    let permit = observer.reserve("status", id, 128).unwrap();
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for index in 0..128 {
        let (sender, receiver) = fifo_channel();
        senders.push(sender);
        receivers.push((format!("{index}:held"), receiver));
    }
    observer
        .observe(permit, "status", id, receivers, true)
        .unwrap();

    let started = Instant::now();
    observer.shutdown();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(observer.snapshot().pending, 0);
    assert_eq!(
        observer.snapshot().last_gap.unwrap().status,
        BackgroundWriteGapStatus::Shutdown
    );
    drop(senders);
}
