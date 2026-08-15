use crate::daemon_harness::*;
use mosaico::{daemon::client::Client, state::Store};
use std::time::Duration;
#[path = "messaging/explicit_destination.rs"]
mod explicit_destination;
#[path = "messaging/inbox_rows.rs"]
mod inbox_rows;
#[path = "messaging/message_coaching.rs"]
mod message_coaching;
#[path = "messaging/non_mention.rs"]
mod non_mention;
#[path = "messaging/self_target.rs"]
mod self_target;
#[path = "messaging/session_start.rs"]
mod session_start;
#[path = "messaging/target_wire.rs"]
mod target_wire;
use inbox_rows::receiver_inbox_rows;

pub(super) fn channel_has_members(channel: &str, expected_pubkeys: &[&str]) -> bool {
    let Ok(response) = mosaico::daemon::blocking::call(
        "channel_members",
        serde_json::json!({ "channel": channel }),
    ) else {
        return false;
    };
    let observed = response["members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|member| member["pubkey"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    expected_pubkeys
        .iter()
        .all(|pubkey| observed.contains(pubkey))
}

pub(super) fn read_messages(params: serde_json::Value) -> Option<Vec<serde_json::Value>> {
    read_channel_messages(params)
}

pub(super) fn observed_message(event_id: &str) -> Option<serde_json::Value> {
    observed_chat(event_id)
}

#[test]
fn observed_send_and_reply_park_direct_inbox_for_owned_recipients() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    crate::channels::write_config(&home, false);

    let (sender_pubkey, receiver_pubkey) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let s = c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "chat-sender", "harness_session": "chat-sender-session", "cwd": "/tmp"}), "claude-code"),
        )
        .await
        .unwrap();
        let r = c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "chat-receiver", "harness_session": "chat-receiver-session", "cwd": "/tmp"}), "claude-code"),
        )
        .await
        .unwrap();
        (
            s["pubkey"].as_str().unwrap().to_string(),
            r["pubkey"].as_str().unwrap().to_string(),
        )
    });
    let store = Store::open(&home.store_path()).unwrap();
    let receiver_row = store
        .get_session(&receiver_pubkey)
        .unwrap()
        .expect("receiver session row");
    let receiver_scope = format!("#{}", only_session_route(&store, &receiver_row.pubkey));
    drop(store);
    assert!(
        wait_until(Duration::from_secs(25), || channel_has_members(
            &receiver_scope,
            &[&sender_pubkey, &receiver_row.pubkey],
        )),
        "sender and receiver did not become relay-confirmed channel members"
    );
    let receiver_pubkey = receiver_row.pubkey.clone();
    let receiver_handle = Store::open(&home.store_path())
        .unwrap()
        .session_identity(&receiver_pubkey)
        .unwrap()
        .expect("receiver identity")
        .display_slug();
    let body = "hello from redirected stdin";
    let wire_body =
        target_wire::redirected_stdin_body_for_session(&home, &receiver_pubkey, &receiver_row);
    let out = run_cli_stdin_with_env_in_dir(
        &home,
        &["channel", "send", "--tag", &receiver_handle],
        &format!("{body}\n"),
        &[("MOSAICO_PUBKEY", &sender_pubkey)],
        std::path::Path::new("/tmp"),
    );
    assert!(
        out.status.success(),
        "channel send failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Publication acceptance is not product delivery. Wait until the daemon's
    // retained NMP observation exposes the message through the public reader.
    let mut original = None;
    assert!(
        wait_until(Duration::from_secs(10), || {
            original = read_messages(serde_json::json!({
                "session": &sender_pubkey,
                "channel": &receiver_scope,
                "limit": 20,
            }))
            .and_then(|messages| {
                messages.into_iter().find(|message| {
                    message["body"]
                        .as_str()
                        .is_some_and(|rendered| rendered.contains(body))
                })
            });
            original.is_some()
        }),
        "observed chat did not reach the public channel reader"
    );
    assert!(
        wait_until(Duration::from_secs(10), || Store::open(&home.store_path())
            .map(|store| receiver_inbox_rows(&store, &receiver_pubkey)
                .iter()
                .any(|row| row.body == wire_body))
            .unwrap_or(false)),
        "observed direct message did not park the recipient inbox"
    );

    // The inbox records the sender's per-session pubkey as `from_pubkey`.
    let sender_pubkey = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sender_pubkey)
        .unwrap()
        .expect("sender session row")
        .pubkey;
    let original_channel = receiver_scope.trim_start_matches('#');
    let original_event_id = original.unwrap()["full_event_id"]
        .as_str()
        .expect("observed message id")
        .to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Store::open(&home.store_path())
        .unwrap()
        .revoke_route_and_mark_absent(&sender_pubkey, original_channel, now + 1)
        .expect("remove reply target's local route");
    let reply_text = "reply delivered after observation";
    let reply_event_id = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let reply = client
            .call(
                "channel_reply",
                serde_json::json!({
                    "session": &receiver_pubkey,
                    "id": original_event_id,
                    "message": reply_text
                }),
            )
            .await
            .expect("publish reply to route-less local author");
        reply["event_id"]
            .as_str()
            .expect("reply event id")
            .to_string()
    });
    assert!(
        wait_until(Duration::from_secs(10), || observed_message(
            &reply_event_id
        )
        .is_some()),
        "accepted reply was never observed through NMP"
    );
    let reply_store = Store::open(&home.store_path()).unwrap();
    assert!(!reply_store
        .has_session_route(&sender_pubkey, original_channel)
        .unwrap());
    drop(reply_store);
    assert!(
        wait_until(Duration::from_secs(10), || Store::open(&home.store_path())
            .map(|store| receiver_inbox_rows(&store, &sender_pubkey)
                .iter()
                .any(|row| row.body.contains(reply_text)))
            .unwrap_or(false)),
        "observed reply did not park under the owned author without a route"
    );
    let store = Store::open(&home.store_path()).unwrap();
    // The inbound routing ledger may still be pending, or may already be marked
    // injected when a live PTY endpoint is present in the integration process.
    let rows = receiver_inbox_rows(&store, &receiver_pubkey);
    let row = rows
        .iter()
        .find(|row| row.body == wire_body)
        .expect("receiver pending chat row");
    assert_eq!(row.target_pubkey, receiver_pubkey);
    assert_eq!(row.from_pubkey, sender_pubkey);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "turn_start",
            serde_json::json!({
                "harness_session": "chat-receiver-session",
                "harness": "claude-code"
            }),
        )
        .await
        .expect("turn_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        store
            .peek_pending_for_pubkey(&sender_pubkey)
            .unwrap()
            .iter()
            .all(|row| row.body != wire_body),
        "sender should not receive its own original chat row"
    );

    stop_daemon(&home);
}
