use crate::fabric::nip29::wire::KIND_CHAT;
use crate::state::{Message, MessageRecipient};

use super::NmpViews;

#[derive(Clone)]
pub(crate) struct MessageProjection {
    pub(crate) message: Message,
    pub(crate) recipients: Vec<MessageRecipient>,
    pub(crate) reply_target: Option<String>,
}

impl NmpViews {
    pub(crate) fn messages(&self) -> Vec<MessageProjection> {
        self.projected_events_for_kind(KIND_CHAT)
            .into_iter()
            .filter_map(|row| {
                let event = row.event;
                if event.channel_h.is_empty() {
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
                        channel_h: event.channel_h,
                        author_pubkey: event.pubkey,
                        body: event.content,
                        created_at: event.created_at,
                        attachment_dir: String::new(),
                    },
                })
            })
            .collect()
    }

    pub(crate) fn message(&self, id: &str) -> Option<MessageProjection> {
        self.messages()
            .into_iter()
            .find(|row| row.message.message_id == id)
    }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nmp::{Row, RowDelta};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::*;

    fn chat() -> Row {
        Row {
            event: EventBuilder::new(Kind::from(KIND_CHAT), "hello")
                .tags([
                    Tag::parse(["h", "room"]).unwrap(),
                    Tag::parse(["p", "peer"]).unwrap(),
                    Tag::parse(["p", "peer"]).unwrap(),
                    Tag::parse(["e", "parent", "", "reply"]).unwrap(),
                ])
                .sign_with_keys(&Keys::generate())
                .unwrap(),
            sources: BTreeSet::new(),
        }
    }

    #[test]
    fn added_and_removed_are_the_only_message_authority() {
        let views = NmpViews::default();
        let row = chat();
        let id = row.event.id;

        views.apply_frame("messages", 1, vec![RowDelta::Added(row)], vec![]);
        let projected = views.message(&id.to_hex()).unwrap();
        assert_eq!(projected.message.body, "hello");
        assert_eq!(projected.recipients.len(), 1);
        assert_eq!(projected.reply_target.as_deref(), Some("parent"));

        views.apply_frame("messages", 1, vec![RowDelta::Removed(id)], vec![]);
        assert!(views.message(&id.to_hex()).is_none());
    }
}
