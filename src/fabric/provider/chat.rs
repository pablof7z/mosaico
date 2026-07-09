use super::Nip29Provider;
use crate::domain::{ChatMessage, DomainEvent};
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate};
use crate::fabric::NostrEventCodec;
use crate::state::{RecordMessage, RelayEvent, Store};
use anyhow::Result;
use nostr_sdk::prelude::{Event, EventId, Keys, Tag};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct OutboundChatRecord {
    pub from_session: Option<String>,
    pub channel_h: String,
    pub body: String,
    pub mentioned_pubkey: Option<String>,
    pub mentioned_session: Option<String>,
    pub created_at: Option<u64>,
    pub direction: &'static str,
}

pub(crate) struct PublishedChat {
    pub event_id: String,
    pub created_at: u64,
}

impl Nip29Provider {
    /// Sign a kind:9 chat event. `reply_to`, when set, appends an `e` tag so the
    /// message threads as a reply to the triggering event — reusing the wire
    /// encoder rather than hand-building a parallel event.
    pub(crate) async fn sign_chat_message(
        &self,
        chat: &ChatMessage,
        reply_to: Option<&str>,
        keys: &Keys,
    ) -> Result<Event> {
        let mut builder = self.wire.encode(&DomainEvent::ChatMessage(chat.clone()))?;
        if let Some(id) = reply_to.filter(|id| !id.is_empty()) {
            builder = builder.tags([Tag::parse(["e", id])?]);
        }
        self.transport.sign(builder, keys).await
    }

    pub(crate) async fn publish_chat_checked(
        &self,
        chat: &ChatMessage,
        keys: &Keys,
        record: &OutboundChatRecord,
    ) -> Result<PublishedChat> {
        let signed = self.sign_chat_message(chat, None, keys).await?;
        self.publish_signed_chat_checked(&signed, record).await
    }

    /// Like [`publish_chat_checked`] but threads the kind:9 as a reply to
    /// `reply_to` via an `e` tag. Used by the turn-end auto-reply path.
    pub(crate) async fn publish_chat_reply_checked(
        &self,
        chat: &ChatMessage,
        reply_to: &str,
        keys: &Keys,
        record: &OutboundChatRecord,
    ) -> Result<PublishedChat> {
        let signed = self.sign_chat_message(chat, Some(reply_to), keys).await?;
        self.publish_signed_chat_checked(&signed, record).await
    }

    pub(crate) async fn publish_signed_chat_checked(
        &self,
        signed: &Event,
        record: &OutboundChatRecord,
    ) -> Result<PublishedChat> {
        let event_id = self
            .publish_signed_chat_event_checked(signed, &record.channel_h)
            .await?;
        let created_at = record
            .created_at
            .unwrap_or_else(|| signed.created_at.as_secs());
        self.with_store(|store| {
            seed_chat_read_models(
                store,
                signed,
                record,
                &event_id.to_hex(),
                created_at,
                "provider_chat_publish",
            )
        });
        Ok(PublishedChat {
            event_id: event_id.to_hex(),
            created_at,
        })
    }

    async fn publish_signed_chat_event_checked(
        &self,
        signed: &Event,
        channel: &str,
    ) -> Result<EventId> {
        let agent_pubkey = signed.pubkey.to_hex();
        let parent = self
            .with_store(|s| s.channel_parent(channel).unwrap_or(None))
            .filter(|p| !p.is_empty());
        let ctx = ChannelCtx {
            channel,
            expect_member: &agent_pubkey,
            parent_hint: parent.as_deref(),
            name: None,
            repair_whitelisted_admins: true,
        };
        if matches!(self.ensure_channel_ready(ctx).await, ChannelGate::Degraded) {
            anyhow::bail!(
                "publish_chat_checked: channel {channel} is not verified (ChannelGate::Degraded) — refusing to publish into an unverified channel"
            );
        }
        self.transport.publish_event_checked(signed).await
    }
}

fn chat_relay_event(
    signed: &Event,
    record: &OutboundChatRecord,
    event_id: &str,
    created_at: u64,
) -> RelayEvent {
    RelayEvent {
        id: event_id.to_string(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: signed.pubkey.to_hex(),
        created_at,
        channel_h: record.channel_h.clone(),
        d_tag: String::new(),
        content: record.body.clone(),
        tags_json: signed_tags_json(signed),
    }
}

fn signed_tags_json(signed: &Event) -> String {
    let raw: Vec<Vec<String>> = signed.tags.iter().map(|t| t.as_slice().to_vec()).collect();
    serde_json::to_string(&raw).unwrap_or_else(|_| "[]".to_string())
}

fn seed_chat_read_models(
    store: &Store,
    signed: &Event,
    record: &OutboundChatRecord,
    event_id: &str,
    created_at: u64,
    context: &str,
) {
    if let Err(e) = store.insert_event(&chat_relay_event(signed, record, event_id, created_at)) {
        tracing::error!(
            event_id,
            channel = %record.channel_h,
            error = %e,
            "{context}: seeding chat into relay_events failed"
        );
    }
    if let Err(e) = store.record_message(&RecordMessage {
        message_id: event_id.to_string(),
        thread_id: record.channel_h.clone(),
        channel_h: record.channel_h.clone(),
        author_pubkey: signed.pubkey.to_hex(),
        author_session: record.from_session.clone(),
        body: record.body.clone(),
        created_at,
        direction: record.direction.to_string(),
        sync_state: "accepted".to_string(),
        native_event_id: Some(event_id.to_string()),
        error: None,
    }) {
        tracing::error!(
            event_id,
            channel = %record.channel_h,
            error = %e,
            "{context}: seeding chat into messages failed"
        );
    }
    if let Some(pk) = &record.mentioned_pubkey {
        if let Err(e) =
            store.add_message_recipient(event_id, pk, record.mentioned_session.as_deref(), None)
        {
            tracing::error!(
                event_id,
                channel = %record.channel_h,
                error = %e,
                "{context}: seeding message recipient failed"
            );
        }
    }
}
