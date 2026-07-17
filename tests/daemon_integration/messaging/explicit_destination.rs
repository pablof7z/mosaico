use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use nostr_sdk::prelude::{Keys, PublicKey, ToBech32};
use std::time::Duration;

#[test]
fn explicit_channel_is_pure_destination_selection_and_preserves_tags() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let (sender, receiver, second_receiver) = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let sender = client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "sender",
                        "harness_session": "explicit-destination-sender",
                        "cwd": "/tmp"
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("start sender");
        let receiver = client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "receiver",
                        "harness_session": "explicit-destination-receiver",
                        "cwd": "/tmp"
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("start receiver");
        let second_receiver = client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "second-receiver",
                        "harness_session": "explicit-destination-second-receiver",
                        "cwd": "/tmp"
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("start second receiver");
        (
            sender["pubkey"].as_str().unwrap().to_string(),
            receiver["pubkey"].as_str().unwrap().to_string(),
            second_receiver["pubkey"].as_str().unwrap().to_string(),
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
        .session_identity(&sender)
        .unwrap()
        .expect("sender identity");
    let receiver_identity = store
        .session_identity(&receiver)
        .unwrap()
        .expect("receiver identity");
    let second_receiver_identity = store
        .session_identity(&second_receiver)
        .unwrap()
        .expect("second receiver identity");
    let receiver_handle = receiver_identity.display_slug();
    let second_receiver_handle = second_receiver_identity.display_slug();
    let receiver_npub = PublicKey::parse(&receiver_identity.pubkey)
        .unwrap()
        .to_bech32()
        .unwrap();
    let second_receiver_npub = PublicKey::parse(&second_receiver_identity.pubkey)
        .unwrap()
        .to_bech32()
        .unwrap();
    drop(store);

    let original_body = "destination-selected message";
    let expected_wire_body = format!(
        "nostr:{receiver_npub}, nostr:{second_receiver_npub}: destination-selected message"
    );
    let sent = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "tmp",
                    "message": original_body,
                    "tags": [&receiver_handle, &second_receiver_handle]
                }),
            )
            .await
            .expect("send to explicitly selected root channel")
    });
    assert_eq!(
        sent["mentioned_pubkeys"],
        serde_json::json!([&receiver_identity.pubkey, &second_receiver_identity.pubkey])
    );
    assert_eq!(
        sent["mentioned_labels"],
        serde_json::json!([&receiver_handle, &second_receiver_handle])
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
    assert!(tags.iter().any(|tag| {
        tag.first().map(String::as_str) == Some("p")
            && tag.get(1).map(String::as_str) == Some(second_receiver_identity.pubkey.as_str())
    }));

    let inline_body = format!("@{receiver_handle}: this stays ambient");
    let guard_error = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "tmp",
                    "message": &inline_body
                }),
            )
            .await
            .expect_err("inline mention text without --tag or --force must fail")
            .to_string()
    });
    assert!(guard_error.contains("did you mean to mention"));
    assert!(guard_error.contains("--tag"));
    assert!(guard_error.contains("--force"));
    let ambient = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "tmp",
                    "message": &inline_body,
                    "force": true
                }),
            )
            .await
            .expect("send inline text without an explicit tag")
    });
    assert_eq!(ambient["mentioned_pubkeys"], serde_json::json!([]));
    let ambient_id = ambient["event_id"].as_str().unwrap().to_string();
    let mut ambient_event = None;
    assert!(
        wait_until(Duration::from_secs(10), || {
            ambient_event = Store::open(&home.store_path()).ok().and_then(|store| {
                chat_in_channel(&store, "tmp")
                    .into_iter()
                    .find(|event| event.id == ambient_id)
            });
            ambient_event.is_some()
        }),
        "ambient inline-address event did not materialize"
    );
    let ambient_event = ambient_event.unwrap();
    assert_eq!(ambient_event.content, inline_body);
    let ambient_tags: Vec<Vec<String>> = serde_json::from_str(&ambient_event.tags_json).unwrap();
    assert!(!ambient_tags
        .iter()
        .any(|tag| tag.first().map(String::as_str) == Some("p")));

    stop_daemon(&home);
}

#[test]
fn channel_commands_require_channel_when_session_joined_to_multiple_channels() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let store = Store::open(&home.store_path()).unwrap();
    store
        .upsert_channel("root-chat-channel", "root-chat-channel", "", "", 1)
        .unwrap();
    store
        .upsert_channel("other-chat-channel", "other-chat-channel", "", "", 1)
        .unwrap();
    let pubkey = Keys::generate().public_key().to_hex();
    store
        .reserve_session_with_facts(
            &mosaico::state::RegisterSession {
                pubkey: pubkey.clone(),
                observed_harness: "codex".to_string(),
                agent_slug: "multi-chat".to_string(),
                channel_h: "root-chat-channel".to_string(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            },
            &mosaico::state::AdmittedRuntimeFacts {
                observed_harness: "codex".into(),
                claimed_harness: "codex".into(),
                bundle: String::new(),
                transport: String::new(),
                endpoint_provenance: "hook".into(),
            },
        )
        .unwrap();
    store
        .join_session_channel(&pubkey, "root-chat-channel", 1)
        .unwrap();
    store
        .join_session_channel(&pubkey, "other-chat-channel", 2)
        .unwrap();

    let write_err = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "message": "ambiguous write",
                    "session": &pubkey
                }),
            )
            .await
            .expect_err("channel send without --channel should fail")
            .to_string()
    });
    assert!(
        write_err.contains("channel send is ambiguous")
            && write_err.contains("mosaico channel send --channel"),
        "unexpected channel send error: {write_err}"
    );

    let read_err = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .stream(
                "channel_read",
                serde_json::json!({
                    "session": &pubkey,
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
            && read_err.contains("mosaico channel read --channel"),
        "unexpected channel read error: {read_err}"
    );

    stop_daemon(&home);
}
