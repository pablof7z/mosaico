use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use nostr::prelude::{
    EventBuilder, EventId, Filter, FinalizeEvent, Keys, Kind, SingleLetterTag, Tag, Timestamp,
};

use super::{database_capacity, NostrRuntime, PublishError};

#[path = "tests/relay.rs"]
mod relay;
use relay::{AckMode, TestRelay};

async fn runtime(relay: &TestRelay, timeout: Duration) -> Arc<NostrRuntime> {
    let runtime = Arc::new(NostrRuntime::with_send_timeout(&relay.url(), timeout).unwrap());
    runtime.start().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !runtime.is_connected().await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("runtime connects");
    runtime
}

#[test]
fn rejects_invalid_relay_and_derives_a_bounded_capacity() {
    assert!(NostrRuntime::new("not a relay").is_err());
    assert_eq!(database_capacity().get(), 6_208);
}

#[tokio::test]
async fn starts_one_connection_and_shuts_it_down() {
    let relay = TestRelay::new(AckMode::Accept, []);
    let runtime = runtime(&relay, Duration::from_millis(100)).await;
    assert_eq!(relay.connections(), 1);
    runtime.shutdown().await;
    assert!(runtime.is_shutdown());
}

#[tokio::test]
async fn explicit_send_distinguishes_accept_reject_and_unconfirmed() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::TextNote, "contract")
        .finalize(&keys)
        .unwrap();

    let accepted_relay = TestRelay::new(AckMode::Accept, []);
    let accepted = runtime(&accepted_relay, Duration::from_millis(100)).await;
    assert_eq!(accepted.publish(&event).await.unwrap(), event.id);

    let rejected_relay = TestRelay::new(AckMode::Reject("x".repeat(400)), []);
    let rejected = runtime(&rejected_relay, Duration::from_millis(100)).await;
    match rejected.publish(&event).await.unwrap_err() {
        PublishError::Rejected { event_id, message } => {
            assert_eq!(event_id, event.id);
            assert!(message.chars().count() <= 256);
        }
        other => panic!("expected rejection, got {other:?}"),
    }

    let silent_relay = TestRelay::new(AckMode::Silent, []);
    let silent = runtime(&silent_relay, Duration::from_millis(40)).await;
    assert!(matches!(
        silent.publish(&event).await.unwrap_err(),
        PublishError::Unconfirmed { event_id, .. } if event_id == event.id
    ));

    let disconnect_relay = TestRelay::new(AckMode::Disconnect, []);
    let disconnected = runtime(&disconnect_relay, Duration::from_millis(40)).await;
    assert!(matches!(
        disconnected.publish(&event).await.unwrap_err(),
        PublishError::Unconfirmed { event_id, .. } if event_id == event.id
    ));
}

#[tokio::test]
async fn active_demands_define_the_complete_raw_level() {
    let keys = Keys::generate();
    let alpha = EventBuilder::new(Kind::TextNote, "alpha")
        .tag(Tag::custom("h", ["alpha"]))
        .finalize(&keys)
        .unwrap();
    let beta = EventBuilder::new(Kind::TextNote, "beta")
        .tag(Tag::custom("h", ["beta"]))
        .finalize(&keys)
        .unwrap();
    let relay = TestRelay::new(AckMode::Accept, [alpha.clone(), beta.clone()]);
    let runtime = runtime(&relay, Duration::from_millis(100)).await;

    runtime
        .replace_demand(
            "content",
            vec![Filter::new()
                .custom_tag(SingleLetterTag::LOWERCASE_H, "alpha")
                .limit(8)],
        )
        .await
        .unwrap();
    wait_for_ids(&runtime, BTreeSet::from([alpha.id])).await;

    runtime
        .replace_demand(
            "content",
            vec![Filter::new()
                .custom_tag(SingleLetterTag::LOWERCASE_H, "beta")
                .limit(8)],
        )
        .await
        .unwrap();
    wait_for_ids(&runtime, BTreeSet::from([beta.id])).await;

    runtime.close_demand("content").await.unwrap();
    wait_for_ids(&runtime, BTreeSet::new()).await;
}

#[tokio::test]
async fn accepted_local_submission_is_not_an_inbound_observation() {
    let relay = TestRelay::new(AckMode::Accept, []);
    let runtime = runtime(&relay, Duration::from_millis(100)).await;
    runtime
        .replace_demand("content", vec![Filter::new().kind(Kind::TextNote).limit(8)])
        .await
        .unwrap();
    let event = EventBuilder::new(Kind::TextNote, "outbound only")
        .finalize(&Keys::generate())
        .unwrap();
    runtime.publish(&event).await.unwrap();
    assert!(runtime.snapshot().await.unwrap().events.is_empty());
}

#[tokio::test]
async fn bounded_database_keeps_only_the_latest_replaceable_value() {
    let keys = Keys::generate();
    let old = EventBuilder::new(Kind::Metadata, r#"{"name":"old"}"#)
        .custom_created_at(Timestamp::from_secs(10))
        .finalize(&keys)
        .unwrap();
    let new = EventBuilder::new(Kind::Metadata, r#"{"name":"new"}"#)
        .custom_created_at(Timestamp::from_secs(20))
        .finalize(&keys)
        .unwrap();
    let relay = TestRelay::new(AckMode::Accept, [old, new.clone()]);
    let runtime = runtime(&relay, Duration::from_millis(100)).await;
    runtime
        .replace_demand(
            "profiles",
            vec![Filter::new()
                .kind(Kind::Metadata)
                .author(keys.public_key())
                .limit(8)],
        )
        .await
        .unwrap();
    runtime
        .wait_for_eose("profiles", Duration::from_secs(1))
        .await
        .unwrap();

    assert_eq!(runtime.snapshot().await.unwrap().events, vec![new]);
}

#[tokio::test]
async fn deletion_markers_retract_content_without_becoming_product_content() {
    let keys = Keys::generate();
    let target = EventBuilder::new(Kind::Custom(9), "delete me")
        .tag(Tag::custom("h", ["alpha"]))
        .custom_created_at(Timestamp::from_secs(10))
        .finalize(&keys)
        .unwrap();
    let deletion = EventBuilder::new(Kind::EventDeletion, "")
        .tags([Tag::event(target.id), Tag::custom("h", ["alpha"])])
        .custom_created_at(Timestamp::from_secs(20))
        .finalize(&keys)
        .unwrap();
    let relay = TestRelay::new(AckMode::Accept, [target, deletion]);
    let runtime = runtime(&relay, Duration::from_millis(100)).await;
    runtime
        .replace_demand(
            "content",
            vec![Filter::new()
                .kinds([Kind::Custom(9), Kind::EventDeletion])
                .custom_tag(SingleLetterTag::LOWERCASE_H, "alpha")
                .limit(8)],
        )
        .await
        .unwrap();
    runtime
        .wait_for_eose("content", Duration::from_secs(1))
        .await
        .unwrap();
    let view = crate::fabric::nip29::view::Nip29ViewStore::default();
    view.replace(runtime.snapshot().await.unwrap().events);

    assert!(view.content_events().is_empty());
}

#[tokio::test]
async fn expired_values_leave_the_complete_raw_level() {
    let keys = Keys::generate();
    let expires = Timestamp::now().as_secs() + 1;
    let event = EventBuilder::new(Kind::Custom(9), "brief")
        .tags([
            Tag::custom("h", ["alpha"]),
            Tag::custom("expiration", [expires.to_string()]),
        ])
        .finalize(&keys)
        .unwrap();
    let relay = TestRelay::new(AckMode::Accept, [event.clone()]);
    let runtime = runtime(&relay, Duration::from_millis(100)).await;
    runtime
        .replace_demand(
            "content",
            vec![Filter::new()
                .kind(Kind::Custom(9))
                .custom_tag(SingleLetterTag::LOWERCASE_H, "alpha")
                .limit(8)],
        )
        .await
        .unwrap();
    wait_for_ids(&runtime, BTreeSet::from([event.id])).await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    assert!(runtime.snapshot().await.unwrap().events.is_empty());
}

async fn wait_for_ids(runtime: &Arc<NostrRuntime>, expected: BTreeSet<EventId>) {
    let mut changes = runtime.subscribe_changes();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let actual = runtime
                .snapshot()
                .await
                .unwrap()
                .events
                .into_iter()
                .map(|event| event.id)
                .collect::<BTreeSet<_>>();
            if actual == expected {
                return;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("expected complete raw level");
}
