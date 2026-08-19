//! Decode retained [`RelayEvent`]s into the chat/reaction projections the
//! state read layer serves.
//!
//! These decoders read only the relay-event fields NMP surfaces
//! (`tags_json`, `content`, `pubkey`, `kind`, `channel_h`, `created_at`,
//! `id`); they do not need the signed event, so [`RecentRows`] — which stores
//! `RelayEvent` — is a sufficient substrate. Status and profile decodes still
//! need the signed event plus the owning channel observation, so they remain
//! on the row mirror (those kinds are replaceable, hence bounded anyway).

use crate::domain::Reaction;
use crate::fabric::nip29::wire::{KIND_CHAT, KIND_REACTION};
use crate::state::{Message, MessageRecipient, RelayEvent};

#[derive(Clone)]
pub(super) struct MessageProjection {
    pub(super) message: Message,
    pub(super) recipients: Vec<MessageRecipient>,
    pub(super) reply_target: Option<String>,
}

#[derive(Clone)]
pub(super) struct ReactionProjection {
    pub(super) reaction_id: String,
    pub(super) target_message_id: String,
    pub(super) channel_h: String,
    pub(super) reactor_pubkey: String,
    pub(super) emoji: String,
    pub(super) created_at: u64,
}

/// Project one kind:9 chat row. Rows with no channel or the wrong kind are
/// skipped, matching the old mirror's `messages()` filter.
pub(super) fn decode_message(event: RelayEvent) -> Option<MessageProjection> {
    if event.kind != KIND_CHAT as u32 || event.channel_h.is_empty() {
        return None;
    }
    let tags = tags(&event.tags_json);
    let mut recipients = tag_values(&tags, "p");
    recipients.sort();
    recipients.dedup();
    Some(MessageProjection {
        recipients: recipients
            .into_iter()
            .map(|recipient_pubkey| MessageRecipient {
                message_id: event.id.clone(),
                recipient_pubkey,
            })
            .collect(),
        reply_target: reply_target(&tags),
        message: Message {
            message_id: event.id,
            channel_h: event.channel_h,
            author_pubkey: event.pubkey,
            body: event.content,
            created_at: event.created_at,
            attachment_dir: String::new(),
        },
    })
}

/// Project one kind:7 reaction row. Malformed emoji or a missing `e` target
/// drops the row, matching the old mirror's `reactions()` filter.
pub(super) fn decode_reaction(event: RelayEvent) -> Option<ReactionProjection> {
    if event.kind != KIND_REACTION as u32 {
        return None;
    }
    let target_message_id = tag_values(&tags(&event.tags_json), "e")
        .into_iter()
        .next()?;
    Reaction::emoji_is_valid(&event.content).then_some(ReactionProjection {
        reaction_id: event.id,
        target_message_id,
        channel_h: event.channel_h,
        reactor_pubkey: event.pubkey,
        emoji: event.content,
        created_at: event.created_at,
    })
}

pub(super) fn tags(tags_json: &str) -> Vec<Vec<String>> {
    serde_json::from_str(tags_json).unwrap_or_default()
}

pub(super) fn tag_values(tags: &[Vec<String>], name: &str) -> Vec<String> {
    tags.iter()
        .filter(|tag| tag.first().map(String::as_str) == Some(name))
        .filter_map(|tag| tag.get(1).filter(|value| !value.is_empty()).cloned())
        .collect()
}

fn reply_target(tags: &[Vec<String>]) -> Option<String> {
    let mut fallback = None;
    for tag in tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("e"))
    {
        let Some(id) = tag.get(1).filter(|id| !id.is_empty()) else {
            continue;
        };
        if tag.get(3).map(String::as_str) == Some("reply") {
            return Some(id.clone());
        }
        fallback = Some(id.clone());
    }
    fallback
}
