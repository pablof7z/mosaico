use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use std::time::Duration;

#[test]
fn agent_cannot_tag_or_reply_to_its_own_identity() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new().with_backend_key();
    crate::channels::write_config(&home, false);
    crate::channels::initialize_workspace_root("tmp", "/tmp");

    let pubkey = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "self-target",
                        "harness_session": "self-target-session",
                        "cwd": "/tmp"
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("start self-target session")["pubkey"]
            .as_str()
            .expect("session pubkey")
            .to_string()
    });
    let handle = Store::open(&home.store_path())
        .unwrap()
        .session_identity(&pubkey)
        .unwrap()
        .expect("session identity")
        .display_slug();
    assert!(
        wait_until(Duration::from_secs(25), || super::channel_has_members(
            "#tmp",
            &[&pubkey],
        )),
        "self-target identity did not become a relay-confirmed /tmp member"
    );

    let tag_error = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &pubkey,
                    "message": "accidental self tag",
                    "tags": [&handle]
                }),
            )
            .await
            .expect_err("self-tag must fail")
            .to_string()
    });
    assert!(
        tag_error.contains("you are trying to --tag yourself")
            && tag_error.contains("tagging yourself is not allowed")
            && tag_error.contains("probably a mistake"),
        "unexpected self-tag error: {tag_error}"
    );

    let event_id = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &pubkey,
                    "message": "message authored by self"
                }),
            )
            .await
            .expect("publish original message")["event_id"]
            .as_str()
            .expect("event id")
            .to_string()
    });
    assert!(
        wait_until(Duration::from_secs(10), || super::observed_message(
            &event_id
        )
        .is_some()),
        "original message was not observed by the public reader"
    );

    let reply_error = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "channel_reply",
                serde_json::json!({
                    "session": &pubkey,
                    "id": &event_id,
                    "message": "accidental self reply"
                }),
            )
            .await
            .expect_err("self-reply must fail")
            .to_string()
    });
    assert!(
        reply_error.contains("you are trying to reply to your own message")
            && reply_error.contains("replying to yourself is not allowed")
            && reply_error.contains("probably a mistake"),
        "unexpected self-reply error: {reply_error}"
    );

    stop_daemon(&home);
}
