use super::*;
use nmp::fifo_channel;

fn event_id(byte: u8) -> EventId {
    EventId::from_slice(&[byte; 32]).unwrap()
}

fn wait_for_failure(observer: &BackgroundReceiptObserver, source_ref: &str) {
    observer
        .evidence
        .wait_for_failure(source_ref, Duration::from_secs(1));
}

#[test]
fn accepted_is_not_terminal_and_acked_records_success() {
    let observer = BackgroundReceiptObserver::start_with(4, 2, Duration::from_secs(1)).unwrap();
    let id = event_id(1);
    let permit = observer.reserve("profile", id, 1).unwrap();
    let (sender, receiver) = fifo_channel();
    assert!(sender.send(WriteStatus::Accepted));
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

    assert!(sender.send(WriteStatus::Signed(id)));
    assert!(sender.send(WriteStatus::Acked(
        nmp::RelayUrl::parse("wss://relay.example.com").unwrap(),
    )));
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
    assert!(sender.send(WriteStatus::Accepted));
    assert!(sender.send(WriteStatus::Signed(id)));
    assert!(sender.send(WriteStatus::Failed(detail.into())));
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
    assert_eq!(evidence.status, BackgroundWriteTerminalStatus::Failed);
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
    let relay = nmp::RelayUrl::parse("wss://relay.example.com").unwrap();
    assert!(sender.send(WriteStatus::PersistenceBlocked(relay.clone())));
    assert!(sender.send(WriteStatus::Acked(relay)));
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
        BackgroundWriteTerminalStatus::PersistenceBlocked
    );
    assert_eq!(
        snapshot.last_success.unwrap().status,
        BackgroundWriteTerminalStatus::Acked
    );
}

#[test]
fn silent_first_stream_does_not_hide_later_failure_for_the_same_event() {
    let observer = BackgroundReceiptObserver::start_with(4, 2, Duration::from_secs(2)).unwrap();
    let id = event_id(3);
    let permit = observer.reserve("profile", id, 2).unwrap();
    let (_held_sender, held_receiver) = fifo_channel();
    let (failed_sender, failed_receiver) = fifo_channel();
    assert!(failed_sender.send(WriteStatus::Failed("second write failed".into())));
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
    assert!(failed_sender.send(WriteStatus::Failed(
        "fifth stream failed immediately".into(),
    )));
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
        assert!(sender.send(WriteStatus::Accepted));
    }
    assert!(!sender.send(WriteStatus::Accepted));
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
