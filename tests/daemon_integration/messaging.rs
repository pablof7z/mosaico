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
#[test]
fn local_send_and_reply_park_direct_inbox_without_waiting_for_relay_echo() {
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
    let receiver_scope = format!("/{}", only_session_route(&store, &receiver_row.pubkey));
    let receiver_channel = receiver_scope.trim_start_matches('/').to_string();
    drop(store);
    assert!(
        wait_until(Duration::from_secs(25), || {
            crate::channels::refresh_channel_members(&receiver_scope);
            Store::open(&home.store_path())
                .map(|store| {
                    store
                        .has_channel_membership_snapshot(&receiver_channel)
                        .unwrap_or(false)
                        && store
                            .is_channel_member(&receiver_channel, &sender_pubkey)
                            .unwrap_or(false)
                        && store
                            .is_channel_member(&receiver_channel, &receiver_row.pubkey)
                            .unwrap_or(false)
                })
                .unwrap_or(false)
        }),
        "sender and receiver did not become relay-confirmed channel members"
    );
    let receiver_pubkey = receiver_row.pubkey.clone();
    let receiver_handle = Store::open(&home.store_path())
        .unwrap()
        .session_identity(&receiver_pubkey)
        .unwrap()
        .expect("receiver identity")
        .display_slug();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_profile(
            &receiver_row.pubkey,
            &receiver_handle,
            &receiver_handle,
            "test-host",
            false,
            now,
        )
        .unwrap();
    let body = "hello from redirected stdin";
    let read_body = target_wire::redirected_stdin_rendered_body(&receiver_handle);
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
    let immediately_parked = Store::open(&home.store_path()).unwrap();
    assert!(
        receiver_inbox_rows(&immediately_parked, &receiver_pubkey)
            .iter()
            .any(|row| row.body == wire_body),
        "successful local send must park the direct inbox before any readback wait"
    );
    drop(immediately_parked);

    // Poll until the relay-materialized chat propagates to the readable store,
    // rather than asserting on a single racy read.
    let mut read_stdout = String::new();
    assert!(
        wait_until(Duration::from_secs(10), || {
            let out = run_cli_with_env_in_dir(
                &home,
                &[
                    "channel",
                    "read",
                    "--channel",
                    &receiver_scope,
                    "--limit",
                    "1",
                ],
                &[("MOSAICO_PUBKEY", &sender_pubkey)],
                std::path::Path::new("/tmp"),
            );
            if !out.status.success() {
                return false;
            }
            read_stdout = String::from_utf8_lossy(&out.stdout).to_string();
            read_stdout.contains(&format!("> {read_body} ["))
        }),
        "channel read should render the body and a timestamp; got: {read_stdout}"
    );

    // The inbox records the sender's per-session pubkey as `from_pubkey`.
    let sender_pubkey = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sender_pubkey)
        .unwrap()
        .expect("sender session row")
        .pubkey;
    let original_channel = receiver_scope.trim_start_matches('/');
    let original_event = Store::open(&home.store_path())
        .unwrap()
        .chat_for_channel(original_channel, 0, u32::MAX)
        .unwrap()
        .into_iter()
        .find(|event| event.content == wire_body)
        .expect("original chat event");
    Store::open(&home.store_path())
        .unwrap()
        .revoke_route_and_mark_absent(&sender_pubkey, original_channel, now + 1)
        .expect("remove reply target's local route");
    let reply_text = "reply parked without relay echo";
    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_reply",
                serde_json::json!({
                    "session": &receiver_pubkey,
                    "id": original_event.id,
                    "message": reply_text
                }),
            )
            .await
            .expect("reply to route-less local author");
    });
    let reply_store = Store::open(&home.store_path()).unwrap();
    assert!(!reply_store
        .has_session_route(&sender_pubkey, original_channel)
        .unwrap());
    assert!(
        receiver_inbox_rows(&reply_store, &sender_pubkey)
            .iter()
            .any(|row| row.body.contains(reply_text)),
        "local reply must park under the owned author even without a target route"
    );
    drop(reply_store);

    assert!(
        wait_until(Duration::from_secs(2), || Store::open(&home.store_path())
            .map(|store| receiver_inbox_rows(&store, &receiver_pubkey)
                .iter()
                .any(|row| row.body == wire_body))
            .unwrap_or(false)),
        "receiver did not get live chat row"
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
        let statusline = c
            .call(
                "statusline",
                serde_json::json!({"session": &receiver_pubkey}),
            )
            .await
            .expect("statusline");
        let pending = statusline["pending"].as_array().expect("pending array");
        // `from_slug` is resolved from the relay-cached profile; the local sender's
        // kind:0 isn't materialized in this nak env, so match on body (the delivery
        // is the invariant; sender identity is checked above via inbox from_pubkey).
        assert!(
            pending.iter().any(|row| { row["body"] == wire_body }),
            "statusline should surface explicit chat mentions as pending: {statusline}"
        );

        c.call(
            "turn_start",
            serde_json::json!({
                "harness_session": "chat-receiver-session",
                "harness": "claude-code"
            }),
        )
        .await
        .expect("turn_start");
        let statusline = c
            .call(
                "statusline",
                serde_json::json!({"session": &receiver_pubkey}),
            )
            .await
            .expect("statusline after drain");
        let recent = statusline["recent"].as_array().expect("recent array");
        assert!(
            recent.iter().any(|row| { row["body"] == wire_body }),
            "statusline should briefly linger drained chat mentions: {statusline}"
        );
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
