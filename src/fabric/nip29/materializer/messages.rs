use super::Nip29Materializer;
use crate::domain::ChatMessage;
use crate::state::{RecordMessage, Store};
use nostr::Event;

impl Nip29Materializer {
    /// Materialise a chat line into the canonical `messages` read model. The
    /// event id is the message id for NIP-29 chat. The event author pubkey is the
    /// sole durable sender identity.
    pub fn materialize_chat_message(store: &Store, event: &Event, chat: &ChatMessage) {
        let channel_h = chat.channel.as_str();
        let from_pubkey = event.pubkey.to_hex();
        let event_id = event.id.to_hex();
        if let Err(e) = store.record_message(&RecordMessage {
            message_id: event_id.clone(),
            thread_id: channel_h.to_string(),
            channel_h: channel_h.to_string(),
            author_pubkey: from_pubkey,
            body: chat.body.clone(),
            created_at: event.created_at.as_secs(),
            sync_state: "accepted".to_string(),
            native_event_id: Some(event_id.clone()),
            error: None,
        }) {
            tracing::error!(
                channel = channel_h,
                event_id = %event_id,
                error = %e,
                "materialize_chat_message: messages upsert failed — channel read model may miss this line"
            );
            return;
        }
        for recipient in &chat.mentioned_pubkeys {
            if let Err(e) = store.add_message_recipient(&event_id, recipient, None) {
                tracing::error!(
                    channel = channel_h,
                    event_id = %event_id,
                    recipient,
                    error = %e,
                    "materialize_chat_message: recipient projection failed"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "messages/tests.rs"]
mod tests;
