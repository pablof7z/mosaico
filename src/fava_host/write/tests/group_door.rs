//! What Mosaico may and may not say about a group, proved at the door.

use super::one_host;
use nostr::event::Tag;
use nostr::event::{EventBuilder, Kind};
use nostr::key::Keys;

/// The group context row belongs to Fava on every path, so a draft that writes
/// its own is REFUSED rather than quietly stripped and re-added. Mosaico used
/// to do the stripping.
#[tokio::test]
async fn a_draft_that_writes_its_own_group_context_row_is_refused() {
    let host = one_host();
    let keys = Keys::generate();
    let builder =
        EventBuilder::new(Kind::TextNote, "hello").tags([Tag::parse(["h", "room-a"]).unwrap()]);

    let error = host
        .publish_group("room-a", builder, keys.public_key())
        .expect_err("the group owns the context row, not the caller");
    assert!(
        error.to_string().contains("belongs to the group"),
        "{error:#}"
    );
}
