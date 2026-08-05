//! Immediate acknowledgement for accepted management commands.

use super::*;
use crate::fabric::nip29::wire::KIND_REACTION;
use anyhow::Result;
use nostr::{EventBuilder, Kind, Tag};

/// Publish a 👍 kind:7 reaction for a management command that has already been
/// parsed, authorized, and durably claimed. This is best-effort: acknowledgement
/// delivery must never prevent the accepted command from executing.
pub(super) async fn publish_thumbs_up(state: &Arc<DaemonState>, event: &Event, channel_h: &str) {
    let event_id = event.id.to_hex();
    let keys = match state.management_keys() {
        Ok(keys) => keys,
        Err(e) => {
            tracing::warn!(
                event_id = %short(&event_id),
                channel = %channel_h,
                error = %e,
                "management command acknowledgement skipped: management key unavailable"
            );
            return;
        }
    };
    let builder = match build_thumbs_up(&event_id) {
        Ok(builder) => builder,
        Err(e) => {
            tracing::warn!(
                event_id = %short(&event_id),
                channel = %channel_h,
                error = %format!("{e:#}"),
                "management command acknowledgement build failed"
            );
            return;
        }
    };
    if let Err(e) = state.nmp.publish_group(channel_h, builder, &keys) {
        tracing::warn!(
            event_id = %short(&event_id),
            channel = %channel_h,
            error = %format!("{e:#}"),
            "management command acknowledgement publish failed"
        );
    }
}

/// The acknowledgement itself. Which group it lands in is the publish door's
/// to say, so no context row is written here.
fn build_thumbs_up(event_id: &str) -> Result<EventBuilder> {
    let e_tag = Tag::parse(["e", event_id])?;
    Ok(EventBuilder::new(Kind::from(KIND_REACTION), "👍").tags([e_tag]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn tag_value<'a>(tags: impl IntoIterator<Item = &'a nostr::Tag>, name: &str) -> Option<String> {
        tags.into_iter().find_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some(name))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
    }

    #[test]
    fn acknowledgement_is_kind_7_thumbs_up_targeting_the_command() {
        let event_id = "ab".repeat(32);
        let unsigned = build_thumbs_up(&event_id)
            .unwrap()
            .build(Keys::generate().public_key());

        assert_eq!(unsigned.kind.as_u16(), KIND_REACTION);
        assert_eq!(unsigned.content, "👍");
        assert_eq!(
            tag_value(unsigned.tags.iter(), "e").as_deref(),
            Some(event_id.as_str())
        );
        // No context row: the group is named at the publish door.
        assert_eq!(tag_value(unsigned.tags.iter(), "h"), None);
    }

    /// The channel the acknowledgement lands in, proved where it is decided.
    #[test]
    fn the_publish_door_puts_the_acknowledgement_in_the_command_channel() {
        let keys = Keys::generate();
        let draft = crate::fabric::nip29::signed_into_group(
            "nmp",
            build_thumbs_up(&"ab".repeat(32)).unwrap(),
            &keys,
        );
        assert_eq!(tag_value(draft.tags.iter(), "h").as_deref(), Some("nmp"));
    }
}
