use super::background_receipts::BackgroundWriteTerminalStatus;
use super::*;
use nmp::fifo_channel;
use nmp::RelayUrl;
use nostr::{EventBuilder, Kind, Tag};
use std::sync::Arc;
use std::time::Duration;

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

#[test]
fn group_template_keeps_product_tags_and_reserves_routing_tags() {
    let tags = [
        Tag::parse(["p", &"a".repeat(64)]).unwrap(),
        Tag::parse(["h", "room-b"]).unwrap(),
        Tag::parse(["h", "room-a"]).unwrap(),
        Tag::parse(["previous", "deadbeef"]).unwrap(),
    ];
    let template = group_template(
        nostr::Timestamp::from(7),
        Kind::TextNote.as_u16(),
        "hello".into(),
        tags.iter().collect(),
    )
    .unwrap();

    assert_eq!(template.group, "room-a");
    assert_eq!(template.extra_tags.len(), 1);
    assert_eq!(template.extra_tags[0][0], "p");
}

#[test]
fn unsigned_multi_group_event_is_rejected_instead_of_losing_scope() {
    let host = NmpHost::open(
        &["wss://relay.example.com".into()],
        None,
        None,
        &Keys::generate(),
    )
    .unwrap();
    let keys = Keys::generate();
    let unsigned = EventBuilder::new(Kind::TextNote, "hello")
        .tags([
            Tag::parse(["h", "room-a"]).unwrap(),
            Tag::parse(["h", "room-b"]).unwrap(),
        ])
        .build(keys.public_key());

    let error = host
        .unsigned_group_intents(&unsigned, keys.public_key())
        .err()
        .expect("multi-group publication must fail");
    assert!(error.to_string().contains("exactly one h tag"));
}

/// The whole optimistic path rests on this: the id returned WITHOUT waiting is
/// the id NMP froze at acceptance. A real engine is used deliberately -- a
/// scripted receipt stream would only prove Mosaico agrees with itself.
#[tokio::test]
async fn the_id_returned_without_waiting_is_the_id_nmp_froze() {
    let host = std::sync::Arc::new(
        NmpHost::open(
            &["wss://relay.example.com".into()],
            None,
            None,
            &Keys::generate(),
        )
        .unwrap(),
    );
    let keys = Keys::generate();
    let builder = EventBuilder::new(Kind::TextNote, "optimistic")
        .tags([Tag::parse(["h", "room-a"]).unwrap()])
        .custom_created_at(nostr::Timestamp::from(1_700_000_000));

    let returned = host.publish_group_builder(builder, &keys).unwrap();

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

/// The durable half of write visibility: an accepted write is readable back
/// out of NMP's own queue, with no receipt id kept and no stream held open.
#[tokio::test]
async fn an_accepted_write_is_visible_in_the_queue_snapshot_without_any_bookkeeping() {
    let host = std::sync::Arc::new(
        NmpHost::open(
            &["wss://relay.example.com".into()],
            None,
            None,
            &Keys::generate(),
        )
        .unwrap(),
    );
    assert_eq!(host.publish_queue_snapshot().outstanding, 0);

    let keys = Keys::generate();
    let builder = EventBuilder::new(Kind::TextNote, "outstanding")
        .tags([Tag::parse(["h", "room-a"]).unwrap()]);
    host.publish_group_builder(builder, &keys).unwrap();

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
    let host = std::sync::Arc::new(
        NmpHost::open(
            &["wss://relay.example.com".into()],
            None,
            None,
            &Keys::generate(),
        )
        .unwrap(),
    );
    let keys = Keys::generate();
    let builder = EventBuilder::new(Kind::TextNote, "no spinner")
        .tags([Tag::parse(["h", "room-a"]).unwrap()]);

    let started = std::time::Instant::now();
    host.publish_group_builder(builder, &keys)
        .expect("acceptance never depends on a relay");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn partial_background_submission_retains_prior_receipts_and_exact_error() {
    fn targeted(index: usize) -> BackgroundIntent {
        let relay =
            RelayUrl::parse(&format!("wss://relay-{index}.example.com")).expect("test relay URL");
        let template = GroupTemplate {
            group: "room".into(),
            created_at: nostr::Timestamp::from(7),
            kind: 1,
            content: "test".into(),
            extra_tags: Vec::new(),
        };
        BackgroundIntent {
            target: format!("{index}:{relay}"),
            intent: group_intent(relay, contextualized_builder(template).unwrap()),
        }
    }

    let (first_sender, first_receiver) = fifo_channel();
    let mut first_receiver = Some(first_receiver);
    let mut call = 0;
    let submission = collect_background_receivers(vec![targeted(0), targeted(1)], |_intent| {
        call += 1;
        if call == 1 {
            Ok(first_receiver.take().unwrap())
        } else {
            Err(anyhow::anyhow!("Previous I/O error occurred").context(
                "intent 1 durable-store persistence failure \
                     [fault=latched durability=absent reopen=required]",
            ))
        }
    });
    let error = submission.error.expect("the second submission must fail");
    let rendered = format!("{error:#}");
    assert!(rendered.contains("fault=latched"), "{rendered}");
    assert!(
        rendered.contains("Previous I/O error occurred"),
        "{rendered}"
    );
    assert_eq!(submission.receivers.len(), 1);

    let observer = BackgroundReceiptObserver::start_with(2, 1, Duration::from_secs(1)).unwrap();
    let id = EventId::from_slice(&[9; 32]).unwrap();
    let permit = observer.reserve("status", id, 2).unwrap();
    observer.submission_failed("status", id, &error);
    assert!(
        first_sender.send(nmp::WriteFact::Signing(nmp::SigningState::Refused {
            reason: "prior receipt retained".into(),
        }))
    );
    observer
        .observe(permit, "status", id, submission.receivers, false)
        .unwrap();
    observer.wait_idle();

    let snapshot = observer.snapshot();
    let failure = snapshot.last_failure.unwrap();
    assert_eq!(failure.source_ref, id.to_hex());
    assert_eq!(
        failure.status,
        BackgroundWriteTerminalStatus::SubmissionFailed
    );
    assert!(
        failure.detail.contains("fault=latched"),
        "{}",
        failure.detail
    );
    assert!(
        failure.detail.contains("Previous I/O error occurred"),
        "{}",
        failure.detail
    );
    assert!(snapshot.recent_failures.iter().any(|evidence| {
        evidence.status == BackgroundWriteTerminalStatus::SignerRefused
            && evidence.detail == "prior receipt retained"
    }));
}
