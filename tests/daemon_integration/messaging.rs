use crate::daemon_harness::*;
use std::time::Duration;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[test]
fn session_start_runs_engine_and_records_alive_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    let session_id = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "session_start",
                serde_json::json!({"agent": "coder", "session_id": "sess-int-1", "cwd": "/tmp"}),
            )
            .await
            .expect("session_start");
        v["session_id"].as_str().unwrap().to_string()
    });
    // The daemon MINTS a canonical id; the harness id "sess-int-1" becomes an
    // alias, never the identity.
    assert_ne!(session_id, "sess-int-1", "daemon must mint a canonical id");

    // The daemon (single writer) wrote an alive session row, resolvable BOTH by
    // the canonical id and (via session_aliases) by the harness id.
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-int-1")
        .unwrap()
        .expect("session row resolvable by harness alias");
    assert_eq!(rec.session_id, session_id, "alias resolves to canonical");
    assert!(rec.alive);
    assert_eq!(rec.agent_slug, "coder");

    // `who` should surface it as a local row keyed by the canonical id.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.unwrap();
        let v = c
            .call(
                "who",
                serde_json::json!({"all": true, "all_projects": true}),
            )
            .await
            .unwrap();
        let rows = v["rows"].as_array().unwrap();
        assert!(
            rows.iter()
                .any(|r| r["session_id"] == session_id.as_str() && r["source"] == "Local"),
            "who rows: {rows:?}"
        );
    });

    stop_daemon(&home);
}

#[test]
fn session_start_replaces_prior_session_for_same_host_pid() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    let pid = std::process::id() as i32;

    let (old_canon, new_canon) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v1 = c
            .call(
                "session_start",
                serde_json::json!({
                    "agent": "claude",
                    "session_id": "old-session",
                    "cwd": "/tmp",
                    "watch_pid": pid
                }),
            )
            .await
            .expect("first session_start");
        let v2 = c
            .call(
                "session_start",
                serde_json::json!({
                    "agent": "claude",
                    "session_id": "new-session",
                    "cwd": "/tmp",
                    "watch_pid": pid
                }),
            )
            .await
            .expect("second session_start");
        (
            v1["session_id"].as_str().unwrap().to_string(),
            v2["session_id"].as_str().unwrap().to_string(),
        )
    });

    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        !store.get_session("old-session").unwrap().unwrap().alive,
        "old session should be marked dead"
    );
    assert!(
        store.get_session("new-session").unwrap().unwrap().alive,
        "new session should remain alive"
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.unwrap();
        let v = c
            .call(
                "who",
                serde_json::json!({"all": true, "all_projects": true}),
            )
            .await
            .unwrap();
        let rows = v["rows"].as_array().unwrap();
        assert!(
            !rows.iter().any(|r| r["session_id"] == old_canon.as_str()),
            "old session leaked into who rows: {rows:?}"
        );
        assert!(
            rows.iter().any(|r| r["session_id"] == new_canon.as_str()),
            "new session missing from who rows: {rows:?}"
        );
    });

    stop_daemon(&home);
}

#[test]
fn send_message_then_inbox_roundtrip_same_machine() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Two sessions of two agents on this machine.
        c.call("session_start", serde_json::json!({"agent": "coder", "session_id": "sess-coder", "cwd": "/tmp"}))
            .await
            .unwrap();
        let rev = c.call("session_start", serde_json::json!({"agent": "reviewer", "session_id": "sess-rev", "cwd": "/tmp"}))
            .await
            .unwrap();
        let rev_canon = rev["session_id"].as_str().unwrap().to_string();

        // coder messages reviewer's session, addressed by the harness alias —
        // the daemon resolves it to the canonical target session.
        let r = c
            .call(
                "send_message",
                serde_json::json!({"recipient": "sess-rev", "message": "please review", "session": "sess-coder"}),
            )
            .await
            .expect("send_message");
        assert_eq!(r["target_session"], rev_canon.as_str(), "got {r}");

        // Give the relay round-trip + demux a moment, then reviewer drains inbox.
        for _ in 0..20 {
            let inbox = c
                .call("inbox", serde_json::json!({"session": "sess-rev"}))
                .await
                .unwrap();
            let rows = inbox["rows"].as_array().unwrap();
            if rows.iter().any(|m| m["body"] == "please review") {
                return; // success
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("reviewer never received the mention");
    });

    stop_daemon(&home);
}

#[test]
fn chat_write_stdin_enqueues_live_project_chat_for_receiver() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    let (sender_canon, receiver_canon) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let s = c.call(
            "session_start",
            serde_json::json!({"agent": "chat-sender", "session_id": "chat-sender-session", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        let r = c.call(
            "session_start",
            serde_json::json!({"agent": "chat-receiver", "session_id": "chat-receiver-session", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        (
            s["session_id"].as_str().unwrap().to_string(),
            r["session_id"].as_str().unwrap().to_string(),
        )
    });

    let out = run_cli_stdin(
        &home,
        &[
            "chat",
            "write",
            "--session",
            "chat-sender-session",
            "--mention",
            "chat-receiver-session",
        ],
        "hello from redirected stdin\n",
    );
    assert!(
        out.status.success(),
        "chat write failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_cli(&home, &["chat", "read", "--project", "tmp", "--limit", "1"]);
    assert!(
        out.status.success(),
        "chat read failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("<chat-sender@test-host> hello from redirected stdin ["),
        "chat read should render sender, host, body, and timestamp; got: {stdout}"
    );

    let mut received = false;
    for _ in 0..12 {
        let store = Store::open(&home.store_path()).unwrap();
        let rows = store.peek_chat(&receiver_canon).unwrap();
        if let Some(row) = rows
            .iter()
            .find(|row| row.body == "hello from redirected stdin")
        {
            assert_eq!(row.mentioned_session, receiver_canon);
            assert_eq!(row.from_session, sender_canon);
            received = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(received, "receiver did not get live chat row");

    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        store.peek_chat(&sender_canon).unwrap().is_empty(),
        "sender should not receive its own chat row"
    );

    stop_daemon(&home);
}

#[test]
fn mention_to_a_does_not_land_in_b_inbox() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Daemon hosts agents A and B (distinct pubkeys), one session each.
        c.call("session_start", serde_json::json!({"agent": "agent-a", "session_id": "sess-aaa", "cwd": "/tmp"}))
            .await
            .unwrap();
        c.call("session_start", serde_json::json!({"agent": "agent-b", "session_id": "sess-bbb", "cwd": "/tmp"}))
            .await
            .unwrap();

        // A third agent (sender) messages A's session specifically.
        c.call("session_start", serde_json::json!({"agent": "sender", "session_id": "sess-send", "cwd": "/tmp"}))
            .await
            .unwrap();
        c.call(
            "send_message",
            serde_json::json!({"recipient": "sess-aaa", "message": "for A only", "session": "sess-send"}),
        )
        .await
        .unwrap();

        // Wait until A receives it.
        let mut a_got = false;
        for _ in 0..20 {
            let inbox = c.call("inbox", serde_json::json!({"session": "sess-aaa"})).await.unwrap();
            if inbox["rows"].as_array().unwrap().iter().any(|m| m["body"] == "for A only") {
                a_got = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(a_got, "agent A should have received the mention");

        // B must NOT have it (routing is by to_pubkey, scoped to A's sessions).
        let b_inbox = c.call("inbox", serde_json::json!({"session": "sess-bbb"})).await.unwrap();
        assert!(
            b_inbox["rows"].as_array().unwrap().is_empty(),
            "agent B inbox should be empty, got {:?}",
            b_inbox["rows"]
        );
    });

    stop_daemon(&home);
}

/// Bug A (sibling-session delivery): two sessions of the SAME agent (one pubkey)
/// on this machine. A→B must land in B's inbox via LOCAL delivery (no relay echo
/// dependency), and must NOT land in the sender A's own inbox.
#[test]
fn sibling_session_mention_lands_in_target_via_local_delivery() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Two sessions of the SAME agent slug → same (agent, machine) pubkey.
        c.call("session_start", serde_json::json!({"agent": "claude", "session_id": "sibling-aaaaaa", "cwd": "/tmp"}))
            .await
            .unwrap();
        let b = c.call("session_start", serde_json::json!({"agent": "claude", "session_id": "sibling-bbbbbb", "cwd": "/tmp"}))
            .await
            .unwrap();
        let b_canon = b["session_id"].as_str().unwrap().to_string();

        // Session A messages sibling session B specifically (by harness alias,
        // resolved to canonical by the daemon).
        let r = c
            .call(
                "send_message",
                serde_json::json!({"recipient": "sibling-bbbbbb", "message": "sibling hello", "session": "sibling-aaaaaa", "agent": "claude"}),
            )
            .await
            .expect("send_message");
        assert_eq!(r["target_session"], b_canon.as_str(), "got {r}");

        // Local delivery is synchronous — B should have it immediately (poll a few
        // times to absorb any scheduling jitter, but no relay round-trip needed).
        let mut b_got = false;
        for _ in 0..8 {
            let inbox = c.call("inbox", serde_json::json!({"session": "sibling-bbbbbb"})).await.unwrap();
            if inbox["rows"].as_array().unwrap().iter().any(|m| m["body"] == "sibling hello") {
                b_got = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(b_got, "sibling session B should have received the mention via local delivery");

        // The sender's own session A must NOT receive its own message.
        let a_inbox = c.call("inbox", serde_json::json!({"session": "sibling-aaaaaa"})).await.unwrap();
        assert!(
            a_inbox["rows"].as_array().unwrap().is_empty(),
            "sender session A inbox should be empty, got {:?}",
            a_inbox["rows"]
        );
    });

    stop_daemon(&home);
}
