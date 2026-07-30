use super::*;
use crate::domain::DomainEvent;
use crate::fabric::nip29::wire::{Nip29WireCodec, KIND_CHAT};
use nostr::{EventBuilder, Keys, Kind, Tag};

#[test]
fn inbound_materialization_preserves_every_explicit_p_tag() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let local = Keys::generate().public_key().to_hex();
    let remote = Keys::generate().public_key().to_hex();
    let event = EventBuilder::new(Kind::from(KIND_CHAT), "hello")
        .tags(vec![
            Tag::parse(["h", "room"]).unwrap(),
            Tag::parse(["p", local.as_str()]).unwrap(),
            Tag::parse(["p", remote.as_str()]).unwrap(),
            Tag::parse(["p", remote.as_str()]).unwrap(),
        ])
        .sign_with_keys(&sender)
        .unwrap();
    let Some(DomainEvent::ChatMessage(chat)) = Nip29WireCodec.decode_event(&event) else {
        panic!("expected chat event");
    };

    Nip29Materializer::materialize_chat_message(&store, &event, &chat);

    let recipients = store.message_recipients(&event.id.to_hex()).unwrap();
    assert_eq!(
        recipients
            .iter()
            .map(|edge| edge.recipient_pubkey.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([local, remote])
    );
}
