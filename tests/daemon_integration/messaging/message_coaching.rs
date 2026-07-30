use crate::daemon_harness::*;
use mosaico::{daemon::client::Client, state::Store};
use nostr::{PublicKey, ToBech32};
use std::time::Duration;

async fn start(client: &mut Client, agent: &str) -> String {
    client
        .call(
            "session_start",
            hook_session_start(
                serde_json::json!({
                    "agent": agent,
                    "harness_session": format!("coaching-{agent}"),
                    "cwd": "/tmp"
                }),
                "claude-code",
            ),
        )
        .await
        .unwrap_or_else(|error| panic!("start {agent}: {error:#}"))["pubkey"]
        .as_str()
        .unwrap()
        .to_string()
}

fn notice<'a>(result: &'a serde_json::Value, code: &str) -> Option<&'a serde_json::Value> {
    result["coaching"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|notice| notice["code"] == code)
}

#[test]
fn send_publishes_then_returns_structured_message_coaching() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new().with_backend_key();
    crate::channels::write_config(&home, false);
    crate::channels::initialize_workspace_root("tmp", "/tmp");
    let (sender, receiver) = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        (
            start(&mut client, "coaching-sender").await,
            start(&mut client, "drift-codex").await,
        )
    });
    assert!(
        wait_until(Duration::from_secs(25), || {
            crate::channels::refresh_channel_members("/tmp");
            Store::open(&home.store_path())
                .map(|store| {
                    store
                        .has_channel_membership_snapshot("tmp")
                        .unwrap_or(false)
                        && store.is_channel_member("tmp", &sender).unwrap_or(false)
                        && store.is_channel_member("tmp", &receiver).unwrap_or(false)
                })
                .unwrap_or(false)
        }),
        "coaching participants did not become relay-confirmed /tmp members"
    );
    let receiver_handle = Store::open(&home.store_path())
        .unwrap()
        .session_identity(&receiver)
        .unwrap()
        .unwrap()
        .display_slug();
    let redundant = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "/tmp",
                    "tags": [&receiver_handle],
                    "message": format!("{receiver_handle}: please review")
                }),
            )
            .await
            .expect("publish redundant-prefix chat")
    });
    assert_eq!(
        notice(&redundant, "redundant_tag_prefix").unwrap()["tagged_agent"],
        receiver_handle
    );
    let receiver_npub = PublicKey::parse(&receiver).unwrap().to_bech32().unwrap();
    let event_id = redundant["event_id"].as_str().unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || Store::open(&home.store_path())
            .map(|store| chat_in_channel(&store, "tmp")
                .iter()
                .any(|event| event.id == event_id
                    && event.content == format!("nostr:{receiver_npub}: please review")))
            .unwrap_or(false)),
        "normalized chat did not materialize"
    );

    let ack = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "/tmp",
                    "message": "Got it!"
                }),
            )
            .await
            .expect("ACK-like chat must still publish")
    });
    assert!(ack["event_id"].as_str().is_some());
    let ack_notice = notice(&ack, "ack_like_chat").expect("ACK coaching");
    assert!(ack_notice["summary"]
        .as_str()
        .unwrap()
        .contains("mosaico channel react <message-id>"));

    let ambient_body = format!("{receiver_handle}: this was meant for you");
    let ambient = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "/tmp",
                    "message": &ambient_body
                }),
            )
            .await
            .expect("untagged name-prefix chat must publish")
    });
    assert_eq!(ambient["mentioned_pubkeys"], serde_json::json!([]));
    let prefix = notice(&ambient, "untagged_agent_prefix").expect("prefix coaching");
    assert_eq!(prefix["matched_agent"], receiver_handle);

    let forced = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &sender,
                    "channel": "/tmp",
                    "message": &ambient_body,
                    "force": true
                }),
            )
            .await
            .expect("forced ambient chat")
    });
    assert!(notice(&forced, "untagged_agent_prefix").is_none());
    stop_daemon(&home);
}
