use super::*;
use nostr::event::{EventBuilder, Kind};
use std::time::Duration;

mod group_door;

pub(super) fn one_host() -> NmpHost {
    NmpHost::open(
        &["wss://relay.example.com".into()],
        None,
        None,
        &Keys::generate(),
    )
    .unwrap()
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
    let full = format!("{error:#}");
    assert!(
        full.contains("explicit publication requires at least one relay")
            || full.contains("no configured NIP-29 group host"),
        "{full}"
    );
}
