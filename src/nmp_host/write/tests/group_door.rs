//! What Mosaico may and may not say about a group, proved at the door.

use super::super::*;
use super::one_host;
use nostr::{EventBuilder, Kind, Tag};

/// The group context row belongs to NMP on every path, so a draft that writes
/// its own is REFUSED rather than quietly stripped and re-added. Mosaico used
/// to do the stripping.
#[test]
fn a_draft_that_writes_its_own_group_context_row_is_refused() {
    let host = one_host();
    let keys = Keys::generate();
    let builder =
        EventBuilder::new(Kind::TextNote, "hello").tags([Tag::parse(["h", "room-a"]).unwrap()]);

    let error = host
        .publish_group("room-a", builder, &keys)
        .expect_err("the group owns the context row, not the caller");
    assert!(
        error.to_string().contains("belongs to the group"),
        "{error:#}"
    );
}

/// A write that is genuinely in SEVERAL groups -- the kind:30315 session
/// status -- goes through the same door at a larger arity. There is no signed
/// path, no hand-minted intent and no per-channel copy: one event, one
/// replaceable coordinate, one `h` row per room.
#[tokio::test]
async fn a_several_group_write_goes_through_the_same_door() {
    let host = one_host();
    let keys = Keys::generate();

    let returned = host
        .publish_groups(
            ["room-a".to_string(), "room-b".to_string()],
            EventBuilder::new(Kind::from(30315u16), "working")
                .tags([Tag::parse(["d", "status"]).unwrap()]),
            &keys,
        )
        .expect("a several-group write is ordinary");

    assert!(host
        .engine
        .publish_queue()
        .unwrap()
        .iter()
        .any(|entry| entry.event_id == returned));
}

/// One group is not a special path that happens to agree with the plural one;
/// it IS the plural one. Both spellings of the same single-group write must
/// therefore reach the queue identically.
#[tokio::test]
async fn one_group_is_the_several_group_door_at_arity_one() {
    let host = one_host();
    let keys = Keys::generate();

    let singular = host
        .publish_group("room-a", EventBuilder::new(Kind::TextNote, "same"), &keys)
        .unwrap();
    let plural = host
        .publish_groups(
            ["room-a".to_string()],
            EventBuilder::new(Kind::TextNote, "same"),
            &keys,
        )
        .unwrap();

    let entries = host.engine.publish_queue().unwrap();
    for id in [singular, plural] {
        assert!(entries.iter().any(|entry| entry.event_id == id));
    }
}

/// NMP signs the context row INTO the bytes. Mosaico never sees the signed
/// event -- it holds only the id -- so this reads the frozen id back out of
/// the queue and asserts it is the id the caller was handed, which is the only
/// thing the app is entitled to know at this point.
#[tokio::test]
async fn the_id_mosaico_holds_names_the_write_nmp_signed() {
    let host = one_host();
    let keys = Keys::generate();

    let returned = host
        .publish_group("room-a", EventBuilder::new(Kind::TextNote, "hi"), &keys)
        .unwrap();

    let entry = host
        .engine
        .publish_queue()
        .unwrap()
        .into_iter()
        .find(|entry| entry.event_id == returned)
        .expect("the returned id names a real queue entry");
    assert_eq!(entry.pubkey, keys.public_key());
}
