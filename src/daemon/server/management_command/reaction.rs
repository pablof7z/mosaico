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
    let builder = match build_thumbs_up(&event_id, channel_h) {
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
    if let Err(e) = state.nmp.publish_group_builder(builder, &keys, false).await {
        tracing::warn!(
            event_id = %short(&event_id),
            channel = %channel_h,
            error = %format!("{e:#}"),
            "management command acknowledgement publish failed"
        );
    }
}

fn build_thumbs_up(event_id: &str, channel_h: &str) -> Result<EventBuilder> {
    let e_tag = Tag::parse(["e", event_id])?;
    let h_tag = Tag::parse(["h", channel_h])?;
    Ok(EventBuilder::new(Kind::from(KIND_REACTION), "👍").tags([e_tag, h_tag]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, UnsignedEvent};

    fn tag_value(event: &UnsignedEvent, name: &str) -> Option<String> {
        event.tags.iter().find_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some(name))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
    }

    #[test]
    fn acknowledgement_is_kind_7_thumbs_up_targeting_command_and_channel() {
        let event_id = "ab".repeat(32);
        let unsigned = build_thumbs_up(&event_id, "nmp")
            .unwrap()
            .build(Keys::generate().public_key());

        assert_eq!(unsigned.kind.as_u16(), KIND_REACTION);
        assert_eq!(unsigned.content, "👍");
        assert_eq!(
            tag_value(&unsigned, "e").as_deref(),
            Some(event_id.as_str())
        );
        assert_eq!(tag_value(&unsigned, "h").as_deref(), Some("nmp"));
    }
}
