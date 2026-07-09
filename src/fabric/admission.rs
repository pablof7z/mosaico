use crate::domain::{AgentRef, ChatMessage, DomainEvent};
use crate::fabric::nip29::materializer::{to_relay_event, Nip29Materializer};
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::fabric::{MaterializationOutcome, NostrEventCodec, RawEnvelope};
use crate::state::Store;
use crate::util::now_secs;
use nostr_sdk::Event;

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
                continue;
            }
        };
        let env = RawEnvelope::Nostr(event.clone());
        let Some(DomainEvent::ChatMessage(chat)) = codec.decode(&env) else {
            let _ = store.remove_quarantined_event(&event_id);
            continue;
        };
        match chat_admission(store, &event) {
            Ok(ChatAdmission::Accepted) => {
                woke |= accept_chat(store, &event, &chat).wake_mentions;
                let _ = store.remove_quarantined_event(&event_id);
            }
            Ok(ChatAdmission::Rejected) => {
                let _ = store.remove_quarantined_event(&event_id);
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
    Nip29Materializer::materialize_event(store, event);
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
    let event_json = match serde_json::to_string(event) {
        Ok(json) => json,
        Err(e) => {
            tracing::error!(
                event_id = %event.id,
                error = %e,
                "quarantine_chat: event serialization failed"
            );
            return;
        }
    };
    if let Err(e) = store.quarantine_event(&to_relay_event(event), &event_json, reason, now_secs())
    {
        tracing::error!(
            event_id = %event.id,
            error = %e,
            "quarantine_chat: quarantine write failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RegisterSession, Store};
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

    fn make_tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).unwrap()
    }

    fn build(keys: &Keys, body: &str, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::from(9u16), body.to_string())
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    fn chat(sender_pk: &str, channel: &str, body: &str, mention: Option<String>) -> ChatMessage {
        ChatMessage {
            from: AgentRef::new(sender_pk, String::new()),
            channel: channel.to_string(),
            body: body.to_string(),
            mentioned_pubkey: mention,
        }
    }

    fn register(store: &Store, pubkey: &str, channel: &str, external_id: &str) -> String {
        store
            .register_session(&RegisterSession {
                harness: "test".into(),
                external_id_kind: "harness_session".into(),
                external_id: external_id.into(),
                agent_pubkey: pubkey.into(),
                agent_slug: external_id.into(),
                channel_h: channel.into(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now: 1,
            })
            .unwrap()
    }

    #[test]
    fn materialize_chat_quarantines_until_membership_snapshots_hydrate() {
        let store = Store::open_memory().unwrap();
        let sender = Keys::generate();
        let receiver = Keys::generate();
        let sender_pk = sender.public_key().to_hex();
        let receiver_pk = receiver.public_key().to_hex();
        let receiver_sid = register(&store, &receiver_pk, "proj", "receiver");

        let event = build(
            &sender,
            "ship it",
            vec![make_tag(&["h", "proj"]), make_tag(&["p", &receiver_pk])],
        );
        let chat = chat(&sender_pk, "proj", "ship it", Some(receiver_pk.clone()));

        let out = materialize_chat(&store, &event, &chat);
        assert!(!out.wake_mentions);
        assert_eq!(store.count_quarantined_events("proj").unwrap(), 1);
        assert!(!store.has_event(&event.id.to_hex()).unwrap());
        assert!(store.get_message(&event.id.to_hex()).unwrap().is_none());

        assert!(!replay_quarantined_chat(&store, "proj"));
        assert_eq!(store.count_quarantined_events("proj").unwrap(), 1);

        store
            .replace_channel_admins("proj", &Vec::<String>::new(), 10)
            .unwrap();
        store
            .replace_channel_members("proj", &[sender_pk, receiver_pk], 11)
            .unwrap();

        assert!(replay_quarantined_chat(&store, "proj"));
        assert_eq!(store.count_quarantined_events("proj").unwrap(), 0);
        assert!(store.has_event(&event.id.to_hex()).unwrap());
        assert_eq!(
            store
                .get_message(&event.id.to_hex())
                .unwrap()
                .unwrap()
                .sync_state,
            "accepted"
        );
        let pending = store.peek_pending_for_session(&receiver_sid).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].body, "ship it");
    }

    #[test]
    fn materialize_chat_rejects_non_member_after_hydration() {
        let store = Store::open_memory().unwrap();
        let sender = Keys::generate();
        let sender_pk = sender.public_key().to_hex();
        let event = build(&sender, "not admitted", vec![make_tag(&["h", "proj"])]);
        let chat = chat(&sender_pk, "proj", "not admitted", None);

        store
            .replace_channel_admins("proj", &Vec::<String>::new(), 10)
            .unwrap();
        store
            .replace_channel_members("proj", &Vec::<String>::new(), 11)
            .unwrap();

        let out = materialize_chat(&store, &event, &chat);
        assert!(out.tail.is_none());
        assert!(!out.wake_mentions);
        assert_eq!(store.count_quarantined_events("proj").unwrap(), 0);
        assert!(!store.has_event(&event.id.to_hex()).unwrap());
        assert!(store.get_message(&event.id.to_hex()).unwrap().is_none());
    }
}
