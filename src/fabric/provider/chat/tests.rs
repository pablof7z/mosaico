use super::*;
use crate::domain::AgentRef;
use crate::state::Store;
use std::sync::{Arc, Mutex};

async fn offline_provider() -> Nip29Provider {
    let nmp = Arc::new(
        crate::nmp_host::NmpHost::open(
            &["wss://relay.example.com".into()],
            None,
            None,
            &Keys::generate(),
        )
        .unwrap(),
    );
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
        attachments: Vec::new(),
    }
}

fn addressed_chat(recipient: &str) -> ChatMessage {
    ChatMessage {
        mentioned_pubkeys: vec![recipient.to_string()],
        ..chat()
    }
}

/// The draft is inspected unsigned because that is all Mosaico ever holds: NMP
/// signs inside its own publish door and hands back an id, never an event.
fn draft_tags(builder: nostr::EventBuilder) -> Vec<Vec<String>> {
    builder
        .build(Keys::generate().public_key())
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect()
}

fn has_tag(tags: &[Vec<String>], name: &str, value: &str) -> bool {
    tags.iter().any(|t| {
        t.first().map(String::as_str) == Some(name) && t.get(1).map(String::as_str) == Some(value)
    })
}

#[tokio::test]
async fn reply_threading_appends_e_tag() {
    let provider = offline_provider().await;
    let reply_to = "a".repeat(64);
    let tags = draft_tags(provider.chat_draft(&chat(), Some(&reply_to)).unwrap());

    assert!(
        has_tag(&tags, "e", &reply_to),
        "reply must thread via an e tag: {tags:?}"
    );
}

#[tokio::test]
async fn reply_threading_keeps_addressing_p_tag() {
    let provider = offline_provider().await;
    let reply_to = "c".repeat(64);
    let requester = "a".repeat(64);
    let tags = draft_tags(
        provider
            .chat_draft(&addressed_chat(&requester), Some(&reply_to))
            .unwrap(),
    );

    assert!(has_tag(&tags, "e", &reply_to), "{tags:?}");
    assert!(has_tag(&tags, "p", &requester), "{tags:?}");
}

#[tokio::test]
async fn no_reply_leaves_no_e_tag() {
    let provider = offline_provider().await;
    let tags = draft_tags(provider.chat_draft(&chat(), None).unwrap());

    assert!(
        !tags
            .iter()
            .any(|t| t.first().map(String::as_str) == Some("e")),
        "a non-reply chat must carry no e tag: {tags:?}"
    );
}

/// The context row belongs to NMP's group door, which appends it before the
/// bytes are signed and REFUSES a draft that supplies its own. A chat draft
/// that wrote an `h` would therefore never publish at all.
#[tokio::test]
async fn a_chat_draft_never_writes_its_own_group_context_row() {
    let provider = offline_provider().await;
    for reply_to in [None, Some("d".repeat(64))] {
        let tags = draft_tags(provider.chat_draft(&chat(), reply_to.as_deref()).unwrap());
        assert!(
            !tags
                .iter()
                .any(|t| t.first().map(String::as_str) == Some("h")),
            "{tags:?}"
        );
    }
}

/// Publishing writes NOTHING to the local store. The message comes back
/// through the group subscription NMP injects the accepted row into (#1182)
/// and reaches `messages` through the materializer, its single writer -- so a
/// row appearing here at publish time would be a second writer racing it.
#[tokio::test]
async fn publishing_a_chat_seeds_no_local_row() {
    let provider = offline_provider().await;
    let keys = Keys::generate();
    let author = keys.public_key().to_hex();
    provider.with_store(|store| {
        store.upsert_channel("chan", "chan", "", "", 1).unwrap();
        store
            .replace_channel_admins("chan", std::slice::from_ref(&author), 2)
            .unwrap();
        store
            .replace_channel_members("chan", std::slice::from_ref(&author), 3)
            .unwrap();
    });
    let published = provider
        .publish_chat_checked(&chat(), &keys)
        .await
        .expect("acceptance never depends on a relay");

    provider.with_store(|store| {
        assert!(store.get_message(&published.event_id).unwrap().is_none());
        assert!(store.get_event(&published.event_id).unwrap().is_none());
    });
}
