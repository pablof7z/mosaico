use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use nostr::Keys;
use std::time::Duration;

async fn start_session(client: &mut Client, agent: &str) -> String {
    client
        .call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": agent,
                    "harness_session": format!("explicit-destination-{agent}"),
                    "cwd": "/tmp"
                }),
                "claude-code",
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("start {agent}: {e:#}"))["pubkey"]
        .as_str()
        .unwrap()
        .to_string()
}

fn await_observed_chat(event_id: &str) -> serde_json::Value {
    let mut found = None;
    assert!(
        wait_until(Duration::from_secs(10), || {
            found = super::observed_message(event_id);
            found.is_some()
        }),
        "chat event {event_id} was not observed by the public reader"
    );
    found.unwrap()
}

#[test]
fn explicit_channel_is_pure_destination_selection_and_preserves_tags() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    let (sender, receiver, second_receiver) = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        (
            start_session(&mut client, "sender").await,
            start_session(&mut client, "receiver").await,
            start_session(&mut client, "second-receiver").await,
        )
    });
    assert!(
        wait_until(Duration::from_secs(25), || super::channel_has_members(
            "#tmp",
            &[&sender, &receiver, &second_receiver],
        )),
        "root channel and participants were not observed before explicit-destination send"
    );
    let routes_before = session_routes(&Store::open(&home.store_path()).unwrap(), &sender);
    let child_path = "#tmp/nip29";
    let created = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_create",
                serde_json::json!({
                    "session": &sender,
                    "channel": "#tmp/nip29",
                    "about": "explicit destination regression"
                }),
            )
            .await
            .expect("create child channel")
    });
    assert_eq!(created["channel"], child_path);
    assert_eq!(created["joined"].as_bool(), Some(true));
    let store = Store::open(&home.store_path()).unwrap();
    let routes_after = session_routes(&store, &sender);
    assert!(routes_before
        .iter()
        .all(|route| routes_after.contains(route)));
    assert_eq!(routes_after.len(), routes_before.len() + 1);
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
    drop(store);

    let original_body = "destination-selected message";
    let sent = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "#tmp",
                    "message": original_body,
                    "tags": [&receiver_handle, &second_receiver_handle]
                }),
            )
            .await
            .expect("send to explicitly selected root channel")
    });
    // The bare workspace name is NOT a channel reference any more.
    let bare = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let params = serde_json::json!({
            "session": &sender, "channel": "tmp", "message": original_body
        });
        client
            .call("channel_send", params)
            .await
            .expect_err("a bare channel name must be rejected")
            .to_string()
    });
    assert!(bare.contains("must be a full path"), "{bare}");
    assert_eq!(
        sent["mentioned_pubkeys"],
        serde_json::json!([&receiver_identity.pubkey, &second_receiver_identity.pubkey])
    );
    assert_eq!(
        sent["mentioned_labels"],
        serde_json::json!([&receiver_handle, &second_receiver_handle])
    );
    let event_id = sent["event_id"].as_str().unwrap().to_string();

    let published = await_observed_chat(&event_id);
    assert_eq!(published["from_pubkey"], sender_identity.pubkey);
    assert_eq!(published["channel"], "#tmp");
    let published_body = published["body"].as_str().expect("published body");
    assert!(published_body.contains(original_body), "{published}");
    assert!(!published_body.contains("[from @"));
    assert!(!published_body.contains(child_path));
    assert_eq!(
        published["recipient_refs"]
            .as_array()
            .expect("observed p-tag recipients")
            .len(),
        2
    );

    let inline_body = format!("@{receiver_handle}: this stays ambient");
    let guard_error = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "#tmp",
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
                    "channel": "#tmp",
                    "message": &inline_body,
                    "force": true
                }),
            )
            .await
            .expect("send inline text without an explicit tag")
    });
    assert_eq!(ambient["mentioned_pubkeys"], serde_json::json!([]));
    let ambient_id = ambient["event_id"].as_str().unwrap().to_string();
    let ambient_event = await_observed_chat(&ambient_id);
    assert_eq!(ambient_event["body"], inline_body);
    assert!(ambient_event["recipient_refs"]
        .as_array()
        .expect("ambient recipient projection")
        .is_empty());
    stop_daemon(&home);
}

#[test]
fn channel_commands_require_channel_when_session_joined_to_multiple_channels() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    crate::channels::write_config(&home, false);
    let other_dir = home.dir.path().join("other-chat-workspace");
    std::fs::create_dir_all(&other_dir).unwrap();
    crate::channels::initialize_workspace_root("root-chat-channel", "/tmp");
    crate::channels::initialize_workspace_root("other-chat-channel", other_dir.to_str().unwrap());
    assert!(
        wait_until(Duration::from_secs(25), || {
            observed_channel_members("#root-chat-channel").is_some()
                && observed_channel_members("#other-chat-channel").is_some()
        }),
        "both ambiguous destination roots must first be delivered through NMP"
    );
    let store = Store::open(&home.store_path()).unwrap();
    let pubkey = Keys::generate().public_key().to_hex();
    store
        .reserve_session_with_facts(
            &mosaico::state::RegisterSession {
                pubkey: pubkey.clone(),
                observed_harness: "codex".to_string(),
                agent_slug: "multi-chat".to_string(),
                launch_channel_h: "root-chat-channel".to_string(),
                work_root: "root-chat-channel".to_string(),
                child_pid: None,
                now: 1,
            },
            &mosaico::state::AdmittedRuntimeFacts {
                observed_harness: "codex".into(),
                claimed_harness: "codex".into(),
                preset: String::new(),
                transport: String::new(),
                endpoint_provenance: "hook".into(),
            },
        )
        .unwrap();
    store
        .grant_session_route(&pubkey, "root-chat-channel", 1)
        .unwrap();
    store
        .grant_session_route(&pubkey, "other-chat-channel", 2)
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
