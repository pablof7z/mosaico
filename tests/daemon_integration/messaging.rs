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
    assert_eq!(session_id, "sess-int-1");

    // The daemon (single writer) wrote an alive session row.
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-int-1")
        .unwrap()
        .expect("session row");
    assert!(rec.alive);
    assert_eq!(rec.agent_slug, "coder");

    // `who` should surface it as a local row.
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
                .any(|r| r["session_id"] == "sess-int-1" && r["source"] == "Local"),
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

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
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
        c.call(
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
            !rows.iter().any(|r| r["session_id"] == "old-session"),
            "old session leaked into who rows: {rows:?}"
        );
        assert!(
            rows.iter().any(|r| r["session_id"] == "new-session"),
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
        c.call("session_start", serde_json::json!({"agent": "reviewer", "session_id": "sess-rev", "cwd": "/tmp"}))
            .await
            .unwrap();

        // coder messages reviewer's session.
        let r = c
            .call(
                "send_message",
                serde_json::json!({"recipient": "sess-rev", "message": "please review", "session": "sess-coder"}),
            )
            .await
            .expect("send_message");
        assert!(r["target_session"] == "sess-rev", "got {r}");

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
        c.call("session_start", serde_json::json!({"agent": "claude", "session_id": "sibling-bbbbbb", "cwd": "/tmp"}))
            .await
            .unwrap();

        // Session A messages sibling session B specifically (by session-id prefix).
        let r = c
            .call(
                "send_message",
                serde_json::json!({"recipient": "sibling-bbbbbb", "message": "sibling hello", "session": "sibling-aaaaaa", "agent": "claude"}),
            )
            .await
            .expect("send_message");
        assert_eq!(r["target_session"], "sibling-bbbbbb", "got {r}");

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
