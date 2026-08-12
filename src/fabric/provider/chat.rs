//! Publishing a kind:9 chat message into a NIP-29 group.
//!
//! **Nothing here writes to the local store.** Mosaico used to seed the chat
//! it had just signed before NMP had delivered it. NMP #1182 ended that need:
//! a locally accepted write is injected into every matching live query, and
//! Mosaico consumes that observed row on the same path as a peer's message.

use super::{ConfirmedGroupScope, Nip29Provider};
use crate::domain::{ChatMessage, DomainEvent};
use crate::fabric::NostrEventCodec;
use anyhow::Result;
use nostr::{EventId, Keys, Tag};

#[cfg(test)]
mod tests;

pub(crate) struct PublishedChat {
    pub event_id: String,
}

impl Nip29Provider {
    /// Compose a kind:9 chat draft. `reply_to`, when set, appends an `e` tag so
    /// the message threads as a reply to the triggering event — reusing the
    /// wire encoder rather than hand-building a parallel event.
    ///
    /// The draft carries no `h` row: the group context belongs to NMP's group
    /// door, which appends it before the bytes are signed and refuses a draft
    /// that supplies its own.
    pub(crate) fn chat_draft(
        &self,
        chat: &ChatMessage,
        reply_to: Option<&str>,
    ) -> Result<nostr::EventBuilder> {
        let mut builder = self.wire.encode(&DomainEvent::ChatMessage(chat.clone()))?;
        if let Some(id) = reply_to.filter(|id| !id.is_empty()) {
            builder = builder.tags([Tag::parse(["e", id])?]);
        }
        Ok(builder)
    }

    pub(crate) async fn publish_chat_checked(
        &self,
        chat: &ChatMessage,
        keys: &Keys,
    ) -> Result<PublishedChat> {
        self.publish_chat(chat, None, keys, None).await
    }

    /// Publish under the exact membership result returned by the NMP readiness
    /// operation that immediately precedes this call.
    ///
    /// The result authorizes only this channel/signer pair. It does not update
    /// Mosaico's group view; that view still changes only when NMP delivers the
    /// relay's current group snapshot.
    pub(crate) async fn publish_chat_after_confirmed_membership(
        &self,
        chat: &ChatMessage,
        keys: &Keys,
        scope: &ConfirmedGroupScope,
    ) -> Result<PublishedChat> {
        self.publish_chat(chat, None, keys, Some(scope)).await
    }

    /// Like [`publish_chat_checked`] but threads the kind:9 as a reply to
    /// `reply_to` via an `e` tag.
    pub(crate) async fn publish_chat_reply_checked(
        &self,
        chat: &ChatMessage,
        reply_to: &str,
        keys: &Keys,
    ) -> Result<PublishedChat> {
        self.publish_chat(chat, Some(reply_to), keys, None).await
    }

    async fn publish_chat(
        &self,
        chat: &ChatMessage,
        reply_to: Option<&str>,
        keys: &Keys,
        confirmed_scope: Option<&ConfirmedGroupScope>,
    ) -> Result<PublishedChat> {
        let channel = chat.channel.as_str();
        if channel.is_empty() {
            anyhow::bail!("a chat message must name the group it is published into");
        }
        let signer = keys.public_key().to_hex();
        if let Some(scope) = confirmed_scope {
            scope.require_membership(channel, &signer)?;
        } else {
            self.verify_publish_scope(channel, &signer, true).await?;
        }
        let created_at = crate::util::now_secs();
        let builder = self
            .chat_draft(chat, reply_to)?
            .custom_created_at(nostr::Timestamp::from(created_at));
        let event_id: EventId = self.nmp.publish_group(channel, builder, keys)?;
        Ok(PublishedChat {
            event_id: event_id.to_hex(),
        })
    }
}
