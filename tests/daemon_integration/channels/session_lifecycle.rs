use super::*;

#[test]
fn session_start_without_tenex_private_key_refuses_unverified_channel() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new(); // default config has NO tenexPrivateKey

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let err = c
            .call(
                "session_start",
                serde_json::json!({"agent": "coder", "session_id": "sess-nogrp", "cwd": "/tmp"}),
            )
            .await
            .expect_err("session_start must fail closed without tenexPrivateKey");
        assert!(
            format!("{err:#}").contains("not verified ready"),
            "unexpected session_start error: {err:#}"
        );
    });

    // Fail-closed: without tenexPrivateKey the daemon cannot sign or verify
    // NIP-29 readiness, so it must not leave a live session pointed at phantom
    // channel state.
    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        store
            .get_session("sess-nogrp")
            .unwrap()
            .map(|rec| !rec.alive)
            .unwrap_or(true),
        "failed readiness must not leave a live session row"
    );

    stop_daemon(&home);
}

/// Regression: a duplicate session-start fired by the offline-agent-mention
/// handler (with a different TENEX_EDGE_CHANNEL env, e.g. "backlog") must NOT
/// overwrite the running session's `channel_h` or add a spurious passive join
/// in `session_channels`. Before the fix, the stale env var stomped the active
/// channel and left the session receiving inbox messages from the wrong channel,
/// causing it to respond there with a cross-channel redirect prefix.
#[test]
fn session_reassert_with_wrong_channel_does_not_corrupt_active_channel() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-reassert");
    let real_channel = unique_session("nostr-multi-platform");
    let stale_channel = unique_session("backlog");

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // First start: engine spawns, channel_h = real_channel.
        c.call(
            "session_start",
            serde_json::json!({
                "agent": "codex",
                "session_id": &sid,
                "cwd": "/tmp",
                "channel": real_channel,
                "watch_pid": std::process::id()
            }),
        )
        .await
        .expect("first session_start");
    });

    // The daemon resolves the channel name to an opaque channel_h id; read it
    // back from the store so subsequent assertions compare against the real id.
    let stored_real_channel = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sid)
        .unwrap()
        .expect("session row after first start")
        .channel_h;
    assert!(
        !stored_real_channel.is_empty(),
        "initial channel must be resolved and stored"
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Re-assert from a duplicate spawn with a DIFFERENT channel (simulates
        // the offline-agent-mention handler spawning a new process with
        // TENEX_EDGE_CHANNEL=stale_channel while the engine is already live).
        c.call(
            "session_start",
            serde_json::json!({
                "agent": "codex",
                "session_id": &sid,
                "cwd": "/tmp",
                "channel": stale_channel,
                "watch_pid": std::process::id()
            }),
        )
        .await
        .expect("re-assert session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session(&sid)
        .unwrap()
        .expect("session row after re-assert");
    assert_eq!(
        rec.channel_h, stored_real_channel,
        "re-assert with wrong channel must NOT overwrite the active channel_h"
    );
    // The session must only be joined to the real channel -- exactly one entry.
    // A spurious re-assert with a different channel used to add a second passive
    // join for the stale channel, leaving two rows in session_channels.
    let joined = store
        .list_session_joined_channels(&sid)
        .expect("list_session_joined_channels");
    assert_eq!(
        joined.len(),
        1,
        "session must have exactly one channel join after a re-assert; got {:?}",
        joined
    );
    assert_eq!(
        joined[0].0, stored_real_channel,
        "the only joined channel must be the real (original) channel"
    );

    stop_daemon(&home);
}
