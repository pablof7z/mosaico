use crate::daemon_harness::*;
use tenex_edge::daemon::client::Client;

#[test]
fn resolves_to_specific_session_when_session_id_is_supplied() {
    // Regression: two sessions of the same agent in the same project must NOT
    // collapse to a single statusline. When the statusline RPC receives an
    // explicit `session` (the canonical id, stamped as `@te_session` on the
    // tmux session by `rpc_session_start`), it must resolve to THAT session,
    // not whichever session is newest for the agent+cwd pair.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Two concurrent same-agent sessions in one project now share the project
    // channel (per-session rooms are off by default), so both need selected
    // ordinal signers derived from the backend key.
    let home = Home::new().with_backend_key();

    // Start two sessions with the same agent + cwd but distinct harness ids.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let a = c
            .call(
                "session_start",
                serde_json::json!({"agent": "claude", "session_id": "sess-a", "cwd": "/tmp"}),
            )
            .await
            .expect("session_start a");
        let b = c
            .call(
                "session_start",
                serde_json::json!({"agent": "claude", "session_id": "sess-b", "cwd": "/tmp"}),
            )
            .await
            .expect("session_start b");
        let canon_a = a["session_id"].as_str().unwrap().to_string();
        let canon_b = b["session_id"].as_str().unwrap().to_string();
        assert_ne!(canon_a, canon_b, "two sessions must mint distinct ids");

        // Statusline with session A's canonical id must show session A.
        let v = c
            .call(
                "statusline",
                serde_json::json!({"session": &canon_a, "agent": "claude", "cwd": "/tmp"}),
            )
            .await
            .expect("statusline A");
        assert_eq!(
            v["session_id"].as_str().unwrap(),
            canon_a,
            "statusline --session A must resolve to session A, not the latest"
        );
        // Statusline with session B's canonical id must show session B.
        let v = c
            .call(
                "statusline",
                serde_json::json!({"session": &canon_b, "agent": "claude", "cwd": "/tmp"}),
            )
            .await
            .expect("statusline B");
        assert_eq!(
            v["session_id"].as_str().unwrap(),
            canon_b,
            "statusline --session B must resolve to session B, not the latest"
        );
        // Statusline with NO session (empty) fails open; harness statusline calls
        // are expected to provide the explicit session id.
        let v = c
            .call(
                "statusline",
                serde_json::json!({"session": "", "agent": "claude", "cwd": "/tmp"}),
            )
            .await
            .expect("statusline fallback");
        assert!(
            v["session_id"].as_str().is_none(),
            "empty --session should not guess between sessions: got {v}"
        );
    });

    stop_daemon(&home);
}
