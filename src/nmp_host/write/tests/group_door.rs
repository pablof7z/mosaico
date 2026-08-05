//! What Mosaico may and may not say about a group, proved at the door.

use super::super::*;
use super::one_host;
use nostr::{EventBuilder, Kind, Tag};

/// Mosaico used to strip and re-add the group context row on its way to NMP,
/// and refused a caller-supplied one only on the unsigned path. Both are now
/// the group door's, on both paths -- so a draft that writes its own `h` is
/// refused rather than quietly overwritten, and the asymmetry is gone.
#[test]
fn a_draft_that_writes_its_own_group_context_row_is_refused() {
    let host = one_host();
    let keys = Keys::generate();
    let builder =
        EventBuilder::new(Kind::TextNote, "hello").tags([Tag::parse(["h", "room-a"]).unwrap()]);

    let error = host
        .publish_group_builder("room-a", builder, &keys)
        .expect_err("the group owns the context row, not the caller");
    assert!(
        error.to_string().contains("belongs to the group"),
        "{error:#}"
    );
}

/// The signed half of the same rule, and the one the old asymmetry let
/// through: an event naming several groups has no single answer to which
/// group is publishing it, and NMP now says so on BOTH mint doors.
#[tokio::test]
async fn a_signed_multi_group_event_is_refused_by_the_group_door() {
    let host = std::sync::Arc::new(one_host());
    let keys = Keys::generate();
    let signed = host
        .sign_event(
            EventBuilder::new(Kind::TextNote, "hello").tags([
                Tag::parse(["h", "room-a"]).unwrap(),
                Tag::parse(["h", "room-b"]).unwrap(),
            ]),
            &keys,
        )
        .await
        .unwrap();

    let error = host
        .enqueue_group_event("room-a", &signed)
        .expect_err("an event in two groups cannot be published through one");
    assert!(error.to_string().contains("more than one"), "{error:#}");
}

/// The one write no group door can mint, and the reason Mosaico still keeps a
/// hand-minted intent (pablof7z/nmp#1281): a kind:30315 session status is
/// genuinely in every channel the session occupies at once.
#[tokio::test]
async fn a_multi_group_status_still_reaches_the_queue_through_its_own_door() {
    let host = std::sync::Arc::new(one_host());
    let keys = Keys::generate();
    let signed = host
        .sign_event(
            EventBuilder::new(Kind::from(30315u16), "working").tags([
                Tag::parse(["d", "status"]).unwrap(),
                Tag::parse(["h", "room-a"]).unwrap(),
                Tag::parse(["h", "room-b"]).unwrap(),
            ]),
            &keys,
        )
        .await
        .unwrap();

    let returned = host.enqueue_multi_group_event(&signed).unwrap();
    assert_eq!(returned, signed.id);
    assert!(host
        .engine
        .publish_queue()
        .unwrap()
        .iter()
        .any(|entry| entry.event_id == returned));
}

/// The group door signs the context row INTO the bytes, so an app that signs
/// its own events never writes one and never has to.
#[tokio::test]
async fn signing_into_a_group_puts_the_context_row_inside_the_signature() {
    let host = std::sync::Arc::new(one_host());
    let keys = Keys::generate();
    let signed = host
        .sign_group_event("room-a", EventBuilder::new(Kind::TextNote, "hi"), &keys)
        .await
        .unwrap();

    assert!(signed.verify().is_ok());
    let groups: Vec<&str> = signed
        .tags
        .iter()
        .filter_map(|tag| {
            let row = tag.as_slice();
            (row.first().map(String::as_str) == Some("h"))
                .then(|| row.get(1))?
                .map(String::as_str)
        })
        .collect();
    assert_eq!(groups, ["room-a"]);
    // And the door accepts what it composed.
    host.enqueue_group_event("room-a", &signed).unwrap();
}
