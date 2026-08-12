use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use std::time::Duration;

/// A chat message with no `@mention` (no p-tag) must remain ambient NMP
/// context and never ring a session's inbox doorbell.
#[test]
fn non_mention_chat_does_not_route_to_inbox() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    crate::channels::write_config(&home, false);
    crate::channels::initialize_workspace_root("tmp", "/tmp");

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

    assert!(
        wait_until(Duration::from_secs(25), || super::channel_has_members(
            "#tmp",
            &[&sender_pubkey, &receiver_pubkey],
        )),
        "sender and receiver did not become relay-confirmed /tmp members"
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
        wait_until(Duration::from_secs(10), || super::read_messages(
            serde_json::json!({
                "session": &sender_pubkey,
                "channel": "#tmp",
                "limit": 20,
            }),
        )
        .is_some_and(|messages| messages
            .iter()
            .any(|message| message["body"] == body))),
        "non-mention message was not observed by the public channel reader"
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
    stop_daemon(&home);
}
