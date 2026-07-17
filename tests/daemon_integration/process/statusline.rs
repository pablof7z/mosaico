use crate::daemon_harness::*;
use mosaico::daemon::client::Client;

#[test]
fn resolves_to_specific_session_when_pubkey_is_supplied() {
    // Regression: two sessions of the same agent in the same channel must NOT
    // collapse to a single statusline. When the statusline RPC receives an
    // explicit public session pubkey, it must resolve to THAT session,
    // not whichever session is newest for the agent+cwd pair.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Two concurrent same-agent sessions in one channel now share the channel
    // channel (per-session rooms are off by default), so both need selected
    // ordinal signers derived from the backend key.
    let home = Home::new().with_backend_key();

    // Start two sessions with the same agent + cwd but distinct harness ids.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let a = c
            .call(
                "session_start",
                hook_session_start(serde_json::json!({"agent": "claude", "harness_session": "sess-a", "cwd": "/tmp"}), "claude-code"),
            )
            .await
            .expect("session_start a");
        let b = c
            .call(
                "session_start",
                hook_session_start(serde_json::json!({"agent": "claude", "harness_session": "sess-b", "cwd": "/tmp"}), "claude-code"),
            )
            .await
            .expect("session_start b");
        let pubkey_a = a["pubkey"].as_str().unwrap().to_string();
        let pubkey_b = b["pubkey"].as_str().unwrap().to_string();
        assert_ne!(pubkey_a, pubkey_b, "two sessions must mint distinct keys");
        let store = mosaico::state::Store::open(&home.store_path()).unwrap();
        let handle_a = store
            .session_identity(&pubkey_a)
            .unwrap()
            .unwrap()
            .display_slug();
        let handle_b = store
            .session_identity(&pubkey_b)
            .unwrap()
            .unwrap()
            .display_slug();

        // Statusline with session A's pubkey must show session A.
        let v = c
            .call("statusline", serde_json::json!({"session": &pubkey_a}))
            .await
            .expect("statusline A");
        assert_eq!(
            v["agent"].as_str().unwrap(),
            handle_a,
            "statusline --session A must resolve to session A, not the latest"
        );
        // Statusline with session B's pubkey must show session B.
        let v = c
            .call("statusline", serde_json::json!({"session": &pubkey_b}))
            .await
            .expect("statusline B");
        assert_eq!(
            v["agent"].as_str().unwrap(),
            handle_b,
            "statusline --session B must resolve to session B, not the latest"
        );
        // Statusline with NO session (empty) fails open; harness statusline calls
        // are expected to provide the explicit session id.
        let v = c
            .call("statusline", serde_json::json!({}))
            .await
            .expect("statusline fallback");
        assert!(
            v.as_object().is_some_and(serde_json::Map::is_empty),
            "empty --session should not guess between sessions: got {v}"
        );
    });

    stop_daemon(&home);
}
