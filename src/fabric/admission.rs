use crate::domain::{AgentRef, ChatMessage, DomainEvent};
use crate::fabric::nip29::materializer::{to_relay_event, Nip29Materializer};
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::fabric::{MaterializationOutcome, NostrEventCodec, RawEnvelope};
use crate::state::Store;
use crate::util::now_secs;
use nostr::Event;

enum ChatAdmission {
    Accepted,
    Unhydrated,
    Rejected,
}

pub(super) fn materialize_chat(
    store: &Store,
    event: &Event,
    chat: &ChatMessage,
) -> MaterializationOutcome {
    match chat_admission(store, event) {
        Ok(ChatAdmission::Accepted) => {
            let out = accept_chat(store, event, chat);
            let _ = store.remove_quarantined_event(&event.id.to_hex());
            out
        }
        Ok(ChatAdmission::Unhydrated) => {
            quarantine_chat(store, event, "membership snapshot not hydrated");
            MaterializationOutcome::default()
        }
        Ok(ChatAdmission::Rejected) => {
            let _ = store.remove_quarantined_event(&event.id.to_hex());
            let _ = store.remove_quarantined_event_arrival(&event.id.to_hex());
            MaterializationOutcome::default()
        }
        Err(e) => {
            tracing::error!(
                event_id = %event.id,
                error = %e,
                "materialize_chat: membership admission failed; quarantining chat"
            );
            quarantine_chat(store, event, "membership admission failed");
            MaterializationOutcome::default()
        }
    }
}

pub(super) fn replay_quarantined_chat(store: &Store, channel_h: &str) -> bool {
    match store.has_channel_membership_snapshot(channel_h) {
        Ok(true) => {}
        Ok(false) => return false,
        Err(e) => {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "replay_quarantined_chat: membership snapshot probe failed"
            );
            return false;
        }
    }

    let rows = match store.quarantined_chat_events_for_channel(channel_h) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "replay_quarantined_chat: quarantine read failed"
            );
            return false;
        }
    };

    let codec = Nip29WireCodec;
    let mut woke = false;
    for (event_id, event_json) in rows {
        let event = match serde_json::from_str::<Event>(&event_json) {
            Ok(event) => event,
            Err(e) => {
                tracing::error!(
                    event_id,
                    error = %e,
                    "replay_quarantined_chat: dropping corrupt quarantined event"
                );
                let _ = store.remove_quarantined_event(&event_id);
                let _ = store.remove_quarantined_event_arrival(&event_id);
                continue;
            }
        };
        let env = RawEnvelope::Nostr(event.clone());
        let Some(DomainEvent::ChatMessage(chat)) = codec.decode(&env) else {
            let _ = store.remove_quarantined_event(&event_id);
            let _ = store.remove_quarantined_event_arrival(&event_id);
            continue;
        };
        match chat_admission(store, &event) {
            Ok(ChatAdmission::Accepted) => {
                woke |= accept_chat(store, &event, &chat).wake_mentions;
                let _ = store.remove_quarantined_event(&event_id);
            }
            Ok(ChatAdmission::Rejected) => {
                let _ = store.remove_quarantined_event(&event_id);
                let _ = store.remove_quarantined_event_arrival(&event_id);
            }
            Ok(ChatAdmission::Unhydrated) => {}
            Err(e) => tracing::error!(
                event_id,
                error = %e,
                "replay_quarantined_chat: admission failed; keeping quarantined"
            ),
        }
    }
    woke
}

fn chat_admission(store: &Store, event: &Event) -> anyhow::Result<ChatAdmission> {
    let channel_h = crate::fabric::nip29::nostr_tag(event, "h").unwrap_or("");
    if channel_h.is_empty() || !store.has_channel_membership_snapshot(channel_h)? {
        return Ok(ChatAdmission::Unhydrated);
    }
    if store.is_channel_member(channel_h, &event.pubkey.to_hex())? {
        Ok(ChatAdmission::Accepted)
    } else {
        Ok(ChatAdmission::Rejected)
    }
}

fn accept_chat(store: &Store, event: &Event, chat: &ChatMessage) -> MaterializationOutcome {
    if let Err(error) = store.activate_quarantined_event(&to_relay_event(event)) {
        tracing::error!(event_id = %event.id, %error, "accepted chat activation failed");
        return MaterializationOutcome::default();
    }
    Nip29Materializer::materialize_chat_message(store, event, chat);

    let sender_pk = event.pubkey.to_hex();
    let resolved_slug = store
        .resolve_slug_for_pubkey(&sender_pk)
        .ok()
        .flatten()
        .unwrap_or_default();
    let enriched = if resolved_slug.is_empty() {
        chat.clone()
    } else {
        ChatMessage {
            from: AgentRef::new(sender_pk, resolved_slug),
            ..chat.clone()
        }
    };
    MaterializationOutcome {
        wake_mentions: Nip29Materializer::route_chat(store, event, &enriched),
        tail: Some(DomainEvent::ChatMessage(enriched)),
    }
}

fn quarantine_chat(store: &Store, event: &Event, reason: &str) {
    let relay_event = to_relay_event(event);
    if let Err(error) = store.reserve_quarantined_event_arrival(&relay_event) {
        tracing::error!(event_id = %event.id, %error, "quarantine arrival reservation failed");
        return;
    }
    let event_json = match serde_json::to_string(event) {
        Ok(json) => json,
        Err(e) => {
            tracing::error!(
                event_id = %event.id,
                error = %e,
                "quarantine_chat: event serialization failed"
            );
            let _ = store.remove_quarantined_event_arrival(&event.id.to_hex());
            return;
        }
    };
    if let Err(e) = store.quarantine_event(&relay_event, &event_json, reason, now_secs()) {
        let _ = store.remove_quarantined_event_arrival(&event.id.to_hex());
        tracing::error!(
            event_id = %event.id,
            error = %e,
            "quarantine_chat: quarantine write failed"
        );
    }
}

#[cfg(test)]
#[path = "admission/tests.rs"]
mod tests;
