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

/// Publishing writes nothing to the local store. A message becomes visible
/// only after NMP delivers the observed row through the group subscription.
#[tokio::test]
async fn publishing_a_chat_seeds_no_local_row() {
    use crate::state::{TestGroup, TestGroupDelivery};

    let provider = offline_provider().await;
    let keys = Keys::generate();
    let author = keys.public_key().to_hex();
    provider.with_store(|store| {
        store.install_test_nmp_group_delivery(TestGroupDelivery::new([TestGroup::new("chan")
            .metadata("chan", "", "", 1)
            .admins(vec![author.clone()])
            .members(vec![author.clone()])]));
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

/// An incomplete acquisition cannot prove that an unlisted signer is absent,
/// but a current roster row is still positive evidence that this signer is a
/// member. Publishing must not start a second readiness/readback loop merely
/// because another acquisition branch is unavailable.
#[tokio::test]
async fn a_current_member_can_publish_while_group_acquisition_is_incomplete() {
    use crate::state::{TestGroup, TestGroupDelivery};

    let provider = offline_provider().await;
    let keys = Keys::generate();
    let author = keys.public_key().to_hex();
    provider.with_store(|store| {
        store.install_test_nmp_group_delivery(TestGroupDelivery::new([TestGroup::new("chan")
            .metadata("chan", "", "", 1)
            .members(vec![author])
            .availability(nmp::nip29::GroupAvailability::SourceUnavailable)]));
    });

    provider
        .publish_chat_checked(&chat(), &keys)
        .await
        .expect("the current positive member row authorizes publishing");
}

/// A successful NMP role mutation is the authority for the command that just
/// performed it. The retained observation remains the only authority for later
/// reads, but it may deliver the relay's resulting roster a moment afterward.
#[tokio::test]
async fn confirmed_membership_can_publish_before_the_retained_view_catches_up() {
    let provider = offline_provider().await;
    let keys = Keys::generate();
    let signer = keys.public_key().to_hex();
    let scope = ConfirmedGroupScope::from_nmp_membership("chan", &signer);

    let error = match provider.publish_chat_checked(&chat(), &keys).await {
        Ok(_) => panic!("the empty retained view invented membership"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("does not exist"), "{error:#}");

    provider
        .publish_chat_after_confirmed_membership(&chat(), &keys, &scope)
        .await
        .expect("the exact terminal NMP membership result authorizes this send");

    let wrong_channel = ChatMessage {
        channel: "other".into(),
        ..chat()
    };
    let error = match provider
        .publish_chat_after_confirmed_membership(&wrong_channel, &keys, &scope)
        .await
    {
        Ok(_) => panic!("membership confirmation escaped its channel"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("does not authorize"),
        "{error:#}"
    );

    let other_keys = Keys::generate();
    let error = match provider
        .publish_chat_after_confirmed_membership(&chat(), &other_keys, &scope)
        .await
    {
        Ok(_) => panic!("membership confirmation escaped its signer"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("does not authorize"),
        "{error:#}"
    );

    provider.with_store(|store| {
        assert!(store.list_channels().unwrap().is_empty());
        assert!(store.events_by_kind(9, 10).unwrap().is_empty());
    });
}
