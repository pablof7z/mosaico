use super::*;
use crate::domain::AgentRef;
use crate::state::Store;
use nostr::{EventBuilder, Kind};
use std::sync::{Arc, Mutex};

async fn offline_provider() -> Nip29Provider {
    let nmp = Arc::new(crate::nmp_host::NmpHost::open(&[], None, None, &Keys::generate()).unwrap());
    let store = Arc::new(Mutex::new(Store::open_memory().unwrap()));
    let mgmt = Keys::generate().secret_key().to_secret_hex();
    Nip29Provider::new(nmp, store, Some(mgmt), None, Vec::new())
}

fn chat() -> ChatMessage {
    ChatMessage {
        from: AgentRef::new("pk", "agent"),
        channel: "chan".into(),
        body: "root cause was a retry storm".into(),
        mentioned_pubkeys: Vec::new(),
    }
}

fn addressed_chat(recipient: &str) -> ChatMessage {
    ChatMessage {
        mentioned_pubkeys: vec![recipient.to_string()],
        ..chat()
    }
}

fn has_tag(event: &Event, name: &str, value: &str) -> bool {
    event.tags.iter().any(|t| {
        let s = t.as_slice();
        s.first().map(String::as_str) == Some(name) && s.get(1).map(String::as_str) == Some(value)
    })
}

#[tokio::test]
async fn reply_threading_appends_e_tag_and_keeps_channel() {
    let provider = offline_provider().await;
    let reply_to = "a".repeat(64);
    let signed = provider
        .sign_chat_message(&chat(), Some(&reply_to), &Keys::generate())
        .await
        .unwrap();

    assert!(
        has_tag(&signed, "e", &reply_to),
        "reply must thread via an e tag: {:?}",
        signed.tags
    );
    assert!(
        has_tag(&signed, "h", "chan"),
        "wire channel h tag must survive reply threading: {:?}",
        signed.tags
    );
}

#[tokio::test]
async fn reply_threading_keeps_addressing_p_tag() {
    let provider = offline_provider().await;
    let reply_to = "c".repeat(64);
    let requester = "a".repeat(64);
    let signed = provider
        .sign_chat_message(
            &addressed_chat(&requester),
            Some(&reply_to),
            &Keys::generate(),
        )
        .await
        .unwrap();

    assert!(
        has_tag(&signed, "e", &reply_to),
        "reply must thread via an e tag: {:?}",
        signed.tags
    );
    assert!(
        has_tag(&signed, "p", &requester),
        "reply must p-tag the requester: {:?}",
        signed.tags
    );
}

#[tokio::test]
async fn no_reply_leaves_no_e_tag() {
    let provider = offline_provider().await;
    let signed = provider
        .sign_chat_message(&chat(), None, &Keys::generate())
        .await
        .unwrap();

    assert!(
        !signed
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(String::as_str) == Some("e")),
        "a non-reply chat must carry no e tag: {:?}",
        signed.tags
    );
}

#[tokio::test]
async fn local_relay_event_preserves_signed_reply_tags() {
    let provider = offline_provider().await;
    let reply_to = "b".repeat(64);
    let signed = provider
        .sign_chat_message(&chat(), Some(&reply_to), &Keys::generate())
        .await
        .unwrap();
    let relay = chat_relay_event(
        &signed,
        &OutboundChatRecord {
            channel_h: "chan".into(),
            direction: "outbound",
        },
        &signed.id.to_hex(),
        signed.created_at.as_secs(),
    );

    let tags: Vec<Vec<String>> = serde_json::from_str(&relay.tags_json).unwrap();
    assert!(
        tags.iter()
            .any(|t| t.first().map(String::as_str) == Some("e")
                && t.get(1).map(String::as_str) == Some(reply_to.as_str())),
        "local relay row must preserve reply e tag: {:?}",
        tags
    );
    assert!(
        tags.iter()
            .any(|t| t.first().map(String::as_str) == Some("h")
                && t.get(1).map(String::as_str) == Some("chan")),
        "local relay row must preserve channel h tag: {:?}",
        tags
    );
}

#[test]
fn local_seed_projects_the_canonical_signed_content() {
    let store = Store::open_memory().unwrap();
    let signed = EventBuilder::new(Kind::from(9u16), "wire-redacted")
        .tags([Tag::parse(["h", "chan"]).unwrap()])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let record = OutboundChatRecord {
        channel_h: "chan".into(),
        direction: "outbound",
    };

    seed_chat_read_models(
        &store,
        &signed,
        &record,
        &signed.id.to_hex(),
        signed.created_at.as_secs(),
        "test",
    );

    assert_eq!(
        store
            .get_event(&signed.id.to_hex())
            .unwrap()
            .unwrap()
            .content,
        signed.content
    );
    assert_eq!(
        store
            .get_message(&signed.id.to_hex())
            .unwrap()
            .unwrap()
            .body,
        signed.content
    );
}

#[test]
fn signed_chat_group_is_exact_and_unambiguous() {
    let keys = Keys::generate();
    let exact = EventBuilder::new(Kind::from(9u16), "hello")
        .tags([Tag::parse(["h", "chan"]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    assert_eq!(signed_group(&exact).unwrap(), "chan");

    let ambiguous = EventBuilder::new(Kind::from(9u16), "hello")
        .tags([
            Tag::parse(["h", "chan-a"]).unwrap(),
            Tag::parse(["h", "chan-b"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    assert!(signed_group(&ambiguous)
        .unwrap_err()
        .to_string()
        .contains("exactly one"));
}
