//! Decode the level's signed events into the rows the view serves: profiles,
//! statuses, relay events, and the chat and reaction projections. Runs once
//! per view replacement.
//!
//! The chat and reaction decoders read only the relay-event fields Nostr runtime surfaces
//! (`tags_json`, `content`, `pubkey`, `kind`, `channel_h`, `created_at`, and
//! `id`).

use nostr::prelude::Event;

use crate::domain::{DomainEvent, Reaction};
use crate::fabric::nip29::wire::{KIND_CHAT, KIND_PROFILE, KIND_REACTION, KIND_STATUS};
use crate::state::{Message, MessageRecipient, Profile, RelayEvent, Status};

#[derive(Clone)]
pub(crate) struct MessageProjection {
    pub(crate) message: Message,
    pub(crate) recipients: Vec<MessageRecipient>,
    pub(crate) reply_target: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ReactionProjection {
    pub(crate) reaction_id: String,
    pub(crate) target_message_id: String,
    pub(crate) channel_h: String,
    pub(crate) reactor_pubkey: String,
    pub(crate) emoji: String,
    pub(crate) created_at: u64,
}

/// Project one kind:9 chat row. Rows with no channel or the wrong kind are
/// skipped, matching the old mirror's `messages()` filter.
pub(super) fn decode_message(event: &RelayEvent) -> Option<MessageProjection> {
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
            message_id: event.id.clone(),
            channel_h: event.channel_h.clone(),
            author_pubkey: event.pubkey.clone(),
            body: event.content.clone(),
            created_at: event.created_at,
            attachment_dir: String::new(),
        },
    })
}

/// Project one kind:7 reaction row. Malformed emoji or a missing `e` target
/// drops the row, matching the old mirror's `reactions()` filter.
pub(super) fn decode_reaction(event: &RelayEvent) -> Option<ReactionProjection> {
    if event.kind != KIND_REACTION as u32 {
        return None;
    }
    let target_message_id = tag_values(&tags(&event.tags_json), "e")
        .into_iter()
        .next()?;
    Reaction::emoji_is_valid(&event.content).then(|| ReactionProjection {
        reaction_id: event.id.clone(),
        target_message_id,
        channel_h: event.channel_h.clone(),
        reactor_pubkey: event.pubkey.clone(),
        emoji: event.content.clone(),
        created_at: event.created_at,
    })
}

fn tags(tags_json: &str) -> Vec<Vec<String>> {
    serde_json::from_str(tags_json).unwrap_or_default()
}

fn tag_values(tags: &[Vec<String>], name: &str) -> Vec<String> {
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

pub(super) fn statuses_from_event(event: &Event) -> Vec<Status> {
    if event.kind.as_u16() != KIND_STATUS {
        return Vec::new();
    }
    let Some(DomainEvent::Status(status)) =
        crate::fabric::nip29::wire::Nip29WireCodec.decode_event(event)
    else {
        return Vec::new();
    };
    let updated_at = event.created_at.as_secs();
    status
        .channels
        .iter()
        .map(|channel_h| Status {
            pubkey: status.agent.pubkey.clone(),
            channel_h: channel_h.clone(),
            slug: status.agent.slug.clone(),
            title: status.title.clone(),
            activity: status.activity.clone(),
            workspace: status.workspace.clone(),
            branch: status.branch.clone(),
            state: status.state,
            state_since: status.state_since,
            last_seen: updated_at,
            updated_at,
            expiration: status.expires_at.unwrap_or(0),
        })
        .collect()
}

pub(super) fn relay_event(event: &Event) -> RelayEvent {
    RelayEvent {
        id: event.id.to_hex(),
        kind: event.kind.as_u16() as u32,
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs(),
        channel_h: crate::fabric::nip29::nostr_tag(event, "h")
            .unwrap_or("")
            .to_string(),
        d_tag: crate::fabric::nip29::nostr_tag(event, "d")
            .unwrap_or("")
            .to_string(),
        content: event.content.clone(),
        tags_json: serde_json::to_string(
            &event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string()),
    }
}

pub(super) fn profile_from_event(event: &Event) -> Option<Profile> {
    if event.kind.as_u16() != KIND_PROFILE {
        return None;
    }
    let DomainEvent::Profile(profile) =
        crate::fabric::nip29::wire::Nip29WireCodec.decode_event(event)?
    else {
        return None;
    };
    Some(Profile {
        pubkey: profile.agent.pubkey,
        name: profile.agent.slug.clone(),
        slug: profile.agent.slug,
        agent_slug: profile.agent_slug,
        host: profile.host,
        is_backend: profile.is_backend,
        agents: profile.agents,
        workspaces: profile.workspaces,
        updated_at: event.created_at.as_secs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RelayEvent;

    fn relay(
        id: &str,
        kind: u32,
        channel: &str,
        pubkey: &str,
        content: &str,
        tags_json: &str,
    ) -> RelayEvent {
        RelayEvent {
            id: id.into(),
            kind,
            pubkey: pubkey.into(),
            created_at: 100,
            channel_h: channel.into(),
            d_tag: String::new(),
            content: content.into(),
            tags_json: tags_json.into(),
        }
    }

    #[test]
    fn decode_message_projects_body_recipients_and_reply_target() {
        let event = relay(
            "m1",
            KIND_CHAT as u32,
            "room",
            "author",
            "hello",
            r#"[["p","alice"],["p","bob"],["p","alice"],["e","parent","","reply"]]"#,
        );
        let projected = decode_message(&event).expect("chat decodes");
        assert_eq!(projected.message.message_id, "m1");
        assert_eq!(projected.message.channel_h, "room");
        assert_eq!(projected.message.author_pubkey, "author");
        assert_eq!(projected.message.body, "hello");
        assert_eq!(projected.reply_target.as_deref(), Some("parent"));
        // Recipients deduped + sorted.
        let recipients: Vec<_> = projected
            .recipients
            .into_iter()
            .map(|r| r.recipient_pubkey)
            .collect();
        assert_eq!(recipients, vec!["alice", "bob"]);
    }

    #[test]
    fn decode_message_falls_back_to_first_e_tag_without_reply_marker() {
        let event = relay(
            "m2",
            KIND_CHAT as u32,
            "room",
            "author",
            "top-level",
            r#"[["e","root"]]"#,
        );
        let projected = decode_message(&event).expect("chat decodes");
        assert_eq!(projected.reply_target.as_deref(), Some("root"));
    }

    #[test]
    fn decode_message_drops_rows_with_no_channel_or_wrong_kind() {
        let no_channel = relay("m3", KIND_CHAT as u32, "", "author", "x", "[]");
        assert!(decode_message(&no_channel).is_none());
        let wrong_kind = relay("m4", KIND_REACTION as u32, "room", "author", "x", "[]");
        assert!(decode_message(&wrong_kind).is_none());
    }

    #[test]
    fn decode_reaction_projects_target_emoji_and_reactor() {
        let event = relay(
            "r1",
            KIND_REACTION as u32,
            "room",
            "reactor",
            "👍",
            r#"[["e","target-msg"]]"#,
        );
        let reaction = decode_reaction(&event).expect("reaction decodes");
        assert_eq!(reaction.reaction_id, "r1");
        assert_eq!(reaction.target_message_id, "target-msg");
        assert_eq!(reaction.channel_h, "room");
        assert_eq!(reaction.reactor_pubkey, "reactor");
        assert_eq!(reaction.emoji, "👍");
    }

    #[test]
    fn decode_reaction_drops_invalid_emoji_or_missing_target() {
        let bad_emoji = relay(
            "r2",
            KIND_REACTION as u32,
            "room",
            "reactor",
            "not an emoji",
            r#"[["e","t"]]"#,
        );
        assert!(decode_reaction(&bad_emoji).is_none());
        let no_target = relay("r3", KIND_REACTION as u32, "room", "reactor", "👍", "[]");
        assert!(decode_reaction(&no_target).is_none());
        let wrong_kind = relay(
            "r4",
            KIND_CHAT as u32,
            "room",
            "reactor",
            "👍",
            r#"[["e","t"]]"#,
        );
        assert!(decode_reaction(&wrong_kind).is_none());
    }
}
