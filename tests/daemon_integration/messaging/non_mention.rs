use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use std::time::Duration;

/// A chat message with NO `@mention` (no p-tag) must NOT route to any session's
/// inbox — it stays in relay_events as ambient context only, never ringing the
/// doorbell. Guards the p-tag-gate behaviour introduced alongside the first-turn
/// compact-notice feature.
#[test]
fn non_mention_chat_does_not_route_to_inbox() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let (sender_pubkey, receiver_pubkey) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let s = c
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "ambient-sender",
                        "harness_session": "ambient-sender-sess",
                        "cwd": "/tmp"
                    }),
                    "claude-code",
                ),
            )
            .await
            .unwrap();
        let r = c
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "ambient-receiver",
                        "harness_session": "ambient-receiver-sess",
                        "cwd": "/tmp"
                    }),
                    "claude-code",
                ),
            )
            .await
            .unwrap();
        (
            s["pubkey"].as_str().unwrap().to_string(),
            r["pubkey"].as_str().unwrap().to_string(),
        )
    });

    // Send only once the sender's channel has materialized, so `channel send`
    // doesn't race relay provisioning (cold-relay readiness stall → ~90s fail).
    let sch = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sender_pubkey)
        .unwrap()
        .expect("sender session row")
        .channel_h;
    assert!(
        wait_until(Duration::from_secs(25), || Store::open(&home.store_path())
            .map(|s| s.get_channel(&sch).unwrap_or(None).is_some())
            .unwrap_or(false)),
        "sender channel did not materialize before send"
    );

    // Write a plain channel message — no @mention in the body.
    let body = "no-mention ambient message for routing test";
    let out = run_cli_stdin_with_env_in_dir(
        &home,
        &["channel", "send"],
        &format!("{body}\n"),
        &[
            ("MOSAICO_AGENT", "ambient-sender"),
            ("MOSAICO_PUBKEY", &sender_pubkey),
        ],
        std::path::Path::new("/tmp"),
    );
    assert!(
        out.status.success(),
        "channel send failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        wait_until(Duration::from_secs(2), || Store::open(&home.store_path())
            .map(|store| {
                chat_in_channel(&store, "tmp")
                    .iter()
                    .any(|event| event.content == body)
            })
            .unwrap_or(false)),
        "non-mention message must be stored in relay_events"
    );

    let store = Store::open(&home.store_path()).unwrap();
    let receiver_pubkey = store.get_session(&receiver_pubkey).unwrap().unwrap().pubkey;
    let sender_pubkey = store.get_session(&sender_pubkey).unwrap().unwrap().pubkey;

    // Inbox for the receiver must be empty — no doorbell should ring.
    assert!(
        store
            .peek_pending_for_pubkey(&receiver_pubkey)
            .unwrap()
            .is_empty(),
        "non-mention message must not route to receiver inbox"
    );
    // Sender never receives its own message either.
    assert!(
        store
            .peek_pending_for_pubkey(&sender_pubkey)
            .unwrap()
            .is_empty(),
        "sender must not receive its own message"
    );
    // The message IS stored in relay_events for ambient context.
    let events = chat_in_channel(&store, "tmp");
    assert!(
        events.iter().any(|e| e.content == body),
        "non-mention message must be stored in relay_events; got {:?}",
        events.iter().map(|e| &e.content).collect::<Vec<_>>()
    );

    stop_daemon(&home);
}
