use crate::daemon_harness::*;
use nostr_sdk::prelude::{PublicKey, ToBech32};
use std::time::Duration;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[test]
fn explicit_channel_is_pure_destination_selection_and_preserves_mentions() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let (sender, receiver) = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let sender = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": "sender",
                    "session_id": "explicit-destination-sender",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("start sender");
        let receiver = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": "receiver",
                    "session_id": "explicit-destination-receiver",
                    "cwd": "/tmp"
                }),
            )
            .await
            .expect("start receiver");
        (
            sender["session_id"].as_str().unwrap().to_string(),
            receiver["session_id"].as_str().unwrap().to_string(),
        )
    });

    assert!(
        wait_until(Duration::from_secs(25), || Store::open(&home.store_path())
            .map(|store| store.get_channel("tmp").unwrap_or(None).is_some())
            .unwrap_or(false)),
        "root channel did not materialize before explicit-destination send"
    );

    let child = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let created = client
            .call(
                "channel_create",
                serde_json::json!({
                    "session": &sender,
                    "name": "nip29",
                    "about": "explicit destination regression"
                }),
            )
            .await
            .expect("create child channel");
        created["child_h"].as_str().unwrap().to_string()
    });

    let store = Store::open(&home.store_path()).unwrap();
    let sender_row = store
        .get_session(&sender)
        .unwrap()
        .expect("sender session row");
    assert_eq!(sender_row.channel_h, child);
    assert_eq!(
        store.list_session_joined_channels(&sender).unwrap().len(),
        2
    );
    let sender_identity = store
        .session_identity_for_session(&sender)
        .unwrap()
        .expect("sender identity");
    let receiver_identity = store
        .session_identity_for_session(&receiver)
        .unwrap()
        .expect("receiver identity");
    let receiver_handle = receiver_identity.display_slug();
    let receiver_npub = PublicKey::parse(&receiver_identity.pubkey)
        .unwrap()
        .to_bech32()
        .unwrap();
    drop(store);

    let original_body = format!("@{receiver_handle} destination-selected message");
    let expected_wire_body = format!("nostr:{receiver_npub} destination-selected message");
    let sent = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "tmp",
                    "message": &original_body
                }),
            )
            .await
            .expect("send to explicitly selected root channel")
    });
    assert_eq!(
        sent["mentioned_pubkey"].as_str(),
        Some(receiver_identity.pubkey.as_str())
    );
    assert_eq!(
        sent["mentioned_label"].as_str(),
        Some(receiver_handle.as_str())
    );
    let event_id = sent["event_id"].as_str().unwrap().to_string();

    let mut published = None;
    assert!(
        wait_until(Duration::from_secs(10), || {
            published = Store::open(&home.store_path()).ok().and_then(|store| {
                chat_in_channel(&store, "tmp")
                    .into_iter()
                    .find(|event| event.id == event_id)
            });
            published.is_some()
        }),
        "explicit-destination event did not materialize"
    );
    let published = published.unwrap();
    assert_eq!(published.pubkey, sender_identity.pubkey);
    assert_eq!(published.channel_h, "tmp");
    assert_eq!(published.content, expected_wire_body);
    assert!(!published.content.contains("[from @"));
    assert!(!published.content.contains(&child));
    let tags: Vec<Vec<String>> = serde_json::from_str(&published.tags_json).unwrap();
    assert!(tags.iter().any(|tag| {
        tag.first().map(String::as_str) == Some("p")
            && tag.get(1).map(String::as_str) == Some(receiver_identity.pubkey.as_str())
    }));

    stop_daemon(&home);
}

#[test]
fn channel_commands_require_channel_when_session_joined_to_multiple_channels() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let store = Store::open(&home.store_path()).unwrap();
    let canonical = store
        .register_session(&tenex_edge::state::RegisterSession {
            harness: "codex".to_string(),
            external_id_kind: "harness_session".to_string(),
            external_id: "multi-chat-session".to_string(),
            agent_pubkey: "pk-multi-chat".to_string(),
            agent_slug: "multi-chat".to_string(),
            channel_h: "root-chat-channel".to_string(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now: 1,
        })
        .unwrap();
    store
        .join_session_channel(&canonical, "root-chat-channel", 1)
        .unwrap();
    store
        .join_session_channel(&canonical, "other-chat-channel", 2)
        .unwrap();

    let write_err = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "message": "ambiguous write",
                    "session": &canonical
                }),
            )
            .await
            .expect_err("channel send without --channel should fail")
            .to_string()
    });
    assert!(
        write_err.contains("channel send is ambiguous")
            && write_err.contains("tenex-edge channel send --channel"),
        "unexpected channel send error: {write_err}"
    );

    let read_err = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .stream(
                "channel_read",
                serde_json::json!({
                    "session": &canonical,
                    "tail": true
                }),
                |_| {},
            )
            .await
            .expect_err("channel read without --channel should fail")
            .to_string()
    });
    assert!(
        read_err.contains("channel read is ambiguous")
            && read_err.contains("tenex-edge channel read --channel"),
        "unexpected channel read error: {read_err}"
    );

    stop_daemon(&home);
}
