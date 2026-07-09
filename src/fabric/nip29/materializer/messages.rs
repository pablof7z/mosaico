use super::{collect_p_pubkeys, Nip29Materializer};
use crate::domain::ChatMessage;
use crate::state::{RecordMessage, Store};
use nostr_sdk::Event;

impl Nip29Materializer {
    /// Materialise a chat line into the canonical `messages` read model. The
    /// event id is the message id for NIP-29 chat. Sender session is derived from
    /// the newest status row for `(author pubkey, channel)` when available.
    pub fn materialize_chat_message(store: &Store, event: &Event, chat: &ChatMessage) {
        let channel_h = chat.channel.as_str();
        let from_pubkey = event.pubkey.to_hex();
        let event_id = event.id.to_hex();
        let author_session = store
            .get_status(&from_pubkey, "", channel_h)
            .ok()
            .flatten()
            .and_then(|st| (!st.session_id.is_empty()).then_some(st.session_id));
        if let Err(e) = store.record_message(&RecordMessage {
            message_id: event_id.clone(),
            thread_id: channel_h.to_string(),
            channel_h: channel_h.to_string(),
            author_pubkey: from_pubkey,
            author_session,
            body: chat.body.clone(),
            created_at: event.created_at.as_secs(),
            direction: "inbound".to_string(),
            sync_state: "accepted".to_string(),
            native_event_id: Some(event_id.clone()),
            error: None,
        }) {
            tracing::error!(
                channel = channel_h,
                event_id = %event_id,
                error = %e,
                "materialize_chat_message: messages upsert failed — chat read model may miss this line"
            );
            return;
        }
        for pk in collect_p_pubkeys(event) {
            if let Err(e) = store.add_message_recipient(&event_id, &pk, None, None) {
                tracing::error!(
                    channel = channel_h,
                    event_id = %event_id,
                    recipient = %crate::util::pubkey_short(&pk),
                    error = %e,
                    "materialize_chat_message: recipient upsert failed"
                );
            }
        }
    }
}
