//! Kind:9 chat projection read through the active Nostr runtime view.
//!
//! Message identity, content, ordering, and `p`-tag recipients belong to Nostr runtime.
//! SQLite retains only the independently keyed local attachment directory.

use super::relay_decode::MessageProjection;
use super::*;
use std::collections::BTreeMap;

mod attachments;
mod wait_cursor;

impl Store {
    /// One chat projection by id, or `None` if the id is absent or not a chat
    /// row in the retained window.
    pub(super) fn message_projection_by_id(&self, message_id: &str) -> Option<MessageProjection> {
        self.view()
            .messages()
            .iter()
            .find(|row| row.message.message_id == message_id)
            .cloned()
    }

    fn with_local_attachment(&self, mut message: Message) -> Result<Message> {
        message.attachment_dir = self.message_attachment_dir(&message.message_id)?;
        Ok(message)
    }

    pub fn get_message_by_prefix(&self, prefix: &str) -> Result<Option<Message>> {
        if prefix.len() >= 64 {
            return self.get_message(prefix);
        }
        let view = self.view();
        let mut matches = view
            .messages()
            .iter()
            .filter(|row| row.message.message_id.starts_with(prefix));
        let first = matches.next();
        if matches.next().is_some() {
            anyhow::bail!("ambiguous id prefix {prefix:?}: matches more than one message");
        }
        first
            .map(|row| self.with_local_attachment(row.message.clone()))
            .transpose()
    }

    pub fn get_message(&self, message_id: &str) -> Result<Option<Message>> {
        self.message_projection_by_id(message_id)
            .map(|row| self.with_local_attachment(row.message))
            .transpose()
    }

    pub fn chat_messages_for_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<Message>> {
        self.messages_matching(limit, |message| {
            message.channel_h == channel_h && message.created_at > since
        })
    }

    pub fn recent_chat_messages_for_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<Message>> {
        let mut messages = self
            .view()
            .messages()
            .iter()
            .map(|row| &row.message)
            .filter(|message| message.channel_h == channel_h && message.created_at > since)
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(|left, right| {
            (&right.created_at, &right.message_id).cmp(&(&left.created_at, &left.message_id))
        });
        self.attach_and_truncate(messages, limit)
    }

    pub fn chat_messages_for_channel_after(
        &self,
        channel_h: &str,
        after_created_at: u64,
        after_id: &str,
        limit: u32,
    ) -> Result<Vec<Message>> {
        self.messages_matching(limit, |message| {
            message.channel_h == channel_h
                && (message.created_at > after_created_at
                    || (message.created_at == after_created_at
                        && message.message_id.as_str() > after_id))
        })
    }

    pub fn recent_chat_messages(&self, since: u64, limit: u32) -> Result<Vec<Message>> {
        let mut messages = self
            .view()
            .messages()
            .iter()
            .map(|row| &row.message)
            .filter(|message| message.created_at >= since)
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(|left, right| {
            (&right.created_at, &right.message_id).cmp(&(&left.created_at, &left.message_id))
        });
        self.attach_and_truncate(messages, limit)
    }

    pub fn latest_message_at_by_channel(&self) -> Result<BTreeMap<String, u64>> {
        let mut latest = BTreeMap::new();
        for row in self.view().messages() {
            latest
                .entry(row.message.channel_h.clone())
                .and_modify(|at: &mut u64| *at = (*at).max(row.message.created_at))
                .or_insert(row.message.created_at);
        }
        Ok(latest)
    }

    pub fn pubkey_has_own_message_after_in_channel(
        &self,
        pubkey: &str,
        channel_h: &str,
        since: u64,
    ) -> Result<bool> {
        Ok(self.view().messages().iter().any(|row| {
            row.message.author_pubkey == pubkey
                && row.message.channel_h == channel_h
                && row.message.created_at > since
        }))
    }

    pub fn should_render_reply_nudge(
        &self,
        channel_h: &str,
        message_id: &str,
        author_pubkey: &str,
        since: u64,
    ) -> Result<bool> {
        if self.has_reaction_from_pubkey_on_message(message_id, author_pubkey)? {
            return Ok(false);
        }
        Ok(!self.pubkey_has_own_message_after_in_channel(author_pubkey, channel_h, since)?)
    }

    pub fn message_recipients(&self, message_id: &str) -> Result<Vec<MessageRecipient>> {
        Ok(self
            .message_projection_by_id(message_id)
            .map(|row| row.recipients)
            .unwrap_or_default())
    }

    pub(super) fn message_projection(&self) -> Result<Vec<(Message, Vec<MessageRecipient>)>> {
        self.view()
            .messages()
            .iter()
            .map(|row| {
                Ok((
                    self.with_local_attachment(row.message.clone())?,
                    row.recipients.clone(),
                ))
            })
            .collect()
    }

    fn messages_matching(
        &self,
        limit: u32,
        include: impl Fn(&Message) -> bool,
    ) -> Result<Vec<Message>> {
        let mut messages = self
            .view()
            .messages()
            .iter()
            .map(|row| &row.message)
            .filter(|message| include(message))
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(|left, right| {
            (&left.created_at, &left.message_id).cmp(&(&right.created_at, &right.message_id))
        });
        self.attach_and_truncate(messages, limit)
    }

    fn attach_and_truncate(&self, mut messages: Vec<Message>, limit: u32) -> Result<Vec<Message>> {
        messages.truncate(limit as usize);
        messages
            .into_iter()
            .map(|message| self.with_local_attachment(message))
            .collect()
    }
}

#[cfg(test)]
mod tests;
