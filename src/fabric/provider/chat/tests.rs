use super::*;
use crate::domain::AgentRef;
use crate::state::Store;
use crate::transport::Transport;
use std::sync::{Arc, Mutex};

async fn offline_provider() -> Nip29Provider {
    let transport = Arc::new(Transport::connect(&[], Keys::generate()).await.unwrap());
    let store = Arc::new(Mutex::new(Store::open_memory().unwrap()));
    let mgmt = Keys::generate().secret_key().to_secret_hex();
    Nip29Provider::new(transport, store, Some(mgmt), None, Vec::new(), &[])
}

fn chat() -> ChatMessage {
    ChatMessage {
        from: AgentRef::new("pk", "agent"),
        project: "chan".into(),
        body: "root cause was a retry storm".into(),
        mentioned_pubkey: None,
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
