use super::*;
use nostr::{EventBuilder, Kind, Tag};
use std::sync::{mpsc, Arc};

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
    let keys = Keys::generate();
    let tags = [
        Tag::parse(["p", &"a".repeat(64)]).unwrap(),
        Tag::parse(["h", "room-b"]).unwrap(),
        Tag::parse(["h", "room-a"]).unwrap(),
        Tag::parse(["previous", "deadbeef"]).unwrap(),
    ];
    let template = group_template(
        keys.public_key(),
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
        .publish_group_unsigned(unsigned, Some(keys.public_key()))
        .unwrap_err();
    assert!(error.to_string().contains("exactly one h tag"));
}

#[test]
fn accepted_and_signed_is_enough_for_a_durable_enqueue() {
    let (tx, rx) = mpsc::channel();
    let id = EventId::from_slice(&[7; 32]).unwrap();
    tx.send(WriteStatus::Accepted).unwrap();
    tx.send(WriteStatus::Signed(id)).unwrap();

    assert_eq!(wait_for_write_blocking(vec![rx], None, false).unwrap(), id);
}

#[test]
fn duplicate_rejection_counts_as_already_converged() {
    let (tx, rx) = mpsc::channel();
    let id = EventId::from_slice(&[8; 32]).unwrap();
    let relay = RelayUrl::parse("wss://relay.example.com").unwrap();
    tx.send(WriteStatus::Accepted).unwrap();
    tx.send(WriteStatus::Signed(id)).unwrap();
    tx.send(WriteStatus::Rejected(
        relay,
        "duplicate: already have event".into(),
    ))
    .unwrap();

    assert_eq!(wait_for_write_blocking(vec![rx], None, true).unwrap(), id);
}
