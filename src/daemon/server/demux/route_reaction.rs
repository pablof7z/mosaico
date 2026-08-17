//! Eye-reaction acknowledgement for routed kind:9 mentions.
//!
//! When a kind:9 chat message is routed to a local agent (a `p`-tagged mention
//! that lands in a live session's inbox), the daemon publishes a NIP-25
//! kind:7 reaction with the 👁 emoji. The reaction carries the channel `h` and
//! the routed event id as an `e` tag, signed by the backend management key, so
//! peers can see the mention was observed and dispatched without waiting for
//! the agent's reply.

use super::*;
use crate::fabric::nip29::wire::KIND_REACTION;
use anyhow::Result;
use nostr::{EventBuilder, Kind, Tag};

/// Publish a 👁 kind:7 reaction acknowledging `event` (a routed kind:9), signed
/// by the backend management key. The reaction's `e` tag is the routed event id
/// and its `h` tag is the channel. Best-effort: a missing management key or a
/// relay rejection is logged and dropped — the routed mention itself is
/// already queued in the inbox, so the agent will still respond.
pub(super) async fn publish_eye_reaction(state: &Arc<DaemonState>, event: &Event) {
    let Some(channel_h) = crate::fabric::nip29::nostr_tag(event, "h").map(str::to_string) else {
        tracing::debug!(
            event_id = %&event.id.to_hex()[..8],
            "route_reaction: kind:9 has no h tag — skipping reaction"
        );
        return;
    };
    let event_id = event.id.to_hex();
    let mgmt_keys = match state.management_keys() {
        Ok(keys) => keys,
        Err(e) => {
            tracing::warn!(
                event_id = %&event_id[..8],
                channel = %channel_h,
                error = %e,
                "route_reaction: management key unavailable — skipping eye reaction"
            );
            return;
        }
    };
    let builder = match build_reaction(&event_id) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                event_id = %&event_id[..8],
                channel = %channel_h,
                error = %format!("{e:#}"),
                "route_reaction: reaction builder failed"
            );
            return;
        }
    };
    if let Err(e) = state
        .snapshot()
        .nmp
        .publish_group(&channel_h, builder, &mgmt_keys)
    {
        tracing::warn!(
            event_id = %&event_id[..8],
            channel = %channel_h,
            error = %format!("{e:#}"),
            "route_reaction: eye reaction publish failed"
        );
    }
}

/// The reaction itself. Which group it lands in is the publish door's to say,
/// so no context row is written here.
fn build_reaction(event_id: &str) -> Result<EventBuilder> {
    let e_tag = tag(&["e", event_id])?;
    Ok(EventBuilder::new(Kind::from(KIND_REACTION), "👁").tags([e_tag]))
}

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn tag_val<'a>(tags: impl IntoIterator<Item = &'a nostr::Tag>, name: &str) -> Option<String> {
        tags.into_iter().find_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some(name) {
                s.get(1).cloned()
            } else {
                None
            }
        })
    }

    #[test]
    fn build_reaction_carries_kind7_eye_targeting_the_command() {
        let event_id = "aa".repeat(32);
        let keys = Keys::generate();
        let unsigned = build_reaction(&event_id).unwrap().build(keys.public_key());
        assert_eq!(unsigned.kind.as_u16(), KIND_REACTION);
        assert_eq!(unsigned.content, "👁");
        assert_eq!(
            tag_val(unsigned.tags.iter(), "e").as_deref(),
            Some(&event_id[..])
        );
        // No context row: the group is named at the publish door, and the
        // draft carrying its own would be refused there.
        assert_eq!(tag_val(unsigned.tags.iter(), "h"), None);
    }

    /// The group the reaction lands in, proved where it is actually decided.
    #[test]
    fn the_publish_door_puts_the_reaction_in_the_routed_channel() {
        let keys = Keys::generate();
        let signed = crate::fabric::nip29::signed_into_group(
            "my-channel",
            build_reaction(&"aa".repeat(32)).unwrap(),
            &keys,
        );
        assert_eq!(
            tag_val(signed.tags.iter(), "h").as_deref(),
            Some("my-channel")
        );
    }
}
