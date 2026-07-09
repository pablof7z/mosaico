use crate::daemon_harness::*;
use std::time::Duration;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[path = "messaging/inbox_rows.rs"]
mod inbox_rows;
use inbox_rows::receiver_inbox_rows;
#[test]
fn session_start_runs_engine_and_records_alive_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

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
            .call("who", serde_json::json!({"all": true, "all_roots": true}))
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
    let home = Home::new().with_backend_key();
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
            .call("who", serde_json::json!({"all": true, "all_roots": true}))
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
fn chat_write_stdin_enqueues_live_channel_chat_for_receiver() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

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
    let receiver_row = Store::open(&home.store_path())
        .unwrap()
        .get_session(&receiver_canon)
        .unwrap()
        .expect("receiver session row");
    let receiver_scope = receiver_row.channel_h.clone();
    // A live session is addressed by @codename@host (a bare @role mention is
    // intercepted by the send-guard). kind:0 isn't materialized back in this nak
    // env, so seed the receiver profile under its codename so the mention resolves.
    let receiver_codename = tenex_edge::util::friendly_short_code(&receiver_canon);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_profile(
            &receiver_row.agent_pubkey,
            &receiver_codename,
            &receiver_codename,
            "test-host",
            false,
            now,
        )
        .unwrap();
    let body = format!("hello @{receiver_codename}@test-host from redirected stdin");
    let out = run_cli_stdin_with_env_in_dir(
        &home,
        &["channel", "send"],
        &format!("{body}\n"),
        &[("TENEX_EDGE_AGENT", "chat-sender")],
        std::path::Path::new("/tmp"),
    );
    assert!(
        out.status.success(),
        "chat write failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
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
        &[("TENEX_EDGE_AGENT", "chat-sender")],
        std::path::Path::new("/tmp"),
    );
    assert!(
        out.status.success(),
        "chat read failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // kind:0 isn't materialized back in this nak env, so assert the body+timestamp
    // render; the sender identity is checked deterministically below via from_pubkey.
    assert!(
        stdout.contains(&format!("> {body} [")),
        "chat read should render the body and a timestamp; got: {stdout}"
    );

    // The inbox records the sender's per-session pubkey as `from_pubkey`.
    let sender_pubkey = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sender_canon)
        .unwrap()
        .expect("sender session row")
        .agent_pubkey;
    assert!(
        wait_until(Duration::from_secs(2), || Store::open(&home.store_path())
            .map(|store| receiver_inbox_rows(&store, &receiver_canon)
                .iter()
                .any(|row| row.body == body))
            .unwrap_or(false)),
        "receiver did not get live chat row"
    );
    let store = Store::open(&home.store_path()).unwrap();
    // The inbound routing ledger may still be pending, or may already be marked
    // injected when a live PTY endpoint is present in the integration process.
    let rows = receiver_inbox_rows(&store, &receiver_canon);
    let row = rows
        .iter()
        .find(|row| row.body == body)
        .expect("receiver pending chat row");
    assert_eq!(row.target_session, receiver_canon);
    assert_eq!(row.from_pubkey, sender_pubkey);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let statusline = c
            .call(
                "statusline",
                serde_json::json!({"session": &receiver_canon}),
            )
            .await
            .expect("statusline");
        let pending = statusline["pending"].as_array().expect("pending array");
        // `from_slug` is resolved from the relay-cached profile; the local sender's
        // kind:0 isn't materialized in this nak env, so match on body (the delivery
        // is the invariant; sender identity is checked above via inbox from_pubkey).
        assert!(
            pending.iter().any(|row| { row["body"] == body }),
            "statusline should surface explicit chat mentions as pending: {statusline}"
        );

        c.call(
            "turn_start",
            serde_json::json!({"session": &receiver_canon}),
        )
        .await
        .expect("turn_start");
        let statusline = c
            .call(
                "statusline",
                serde_json::json!({"session": &receiver_canon}),
            )
            .await
            .expect("statusline after drain");
        let recent = statusline["recent"].as_array().expect("recent array");
        assert!(
            recent.iter().any(|row| { row["body"] == body }),
            "statusline should briefly linger drained chat mentions: {statusline}"
        );
    });

    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        store
            .peek_pending_for_session(&sender_canon)
            .unwrap()
            .is_empty(),
        "sender should not receive its own chat row"
    );

    stop_daemon(&home);
}

#[test]
fn chat_commands_require_channel_when_session_joined_to_multiple_channels() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let store = Store::open(&home.store_path()).unwrap();
    let canonical = store
        .register_session(&tenex_edge::state::RegisterSession {
            harness: "codex".to_string(),
            external_id_kind: "harness_session".to_string(),
            external_id: "multi-chat-session".to_string(),
            agent_pubkey: "pk-multi-chat".to_string(),
            agent_slug: "multi-chat".to_string(),
            channel_h: "root-chat-channel".to_string(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now: 1,
        })
        .unwrap();
    store
        .join_session_channel(&canonical, "root-chat-channel", 1)
        .unwrap();
    store
        .join_session_channel(&canonical, "other-chat-channel", 2)
        .unwrap();

    let write_err = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "chat_write",
            serde_json::json!({
                "message": "ambiguous write",
                "session": &canonical
            }),
        )
        .await
        .expect_err("chat write without --channel should fail")
        .to_string()
    });
    assert!(
        write_err.contains("channel send is ambiguous")
            && write_err.contains("tenex-edge channel send --channel"),
        "unexpected chat write error: {write_err}"
    );

    let read_err = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.stream(
            "chat_read",
            serde_json::json!({
                "session": &canonical,
                "tail": true
            }),
            |_| {},
        )
        .await
        .expect_err("chat read without --channel should fail")
        .to_string()
    });
    assert!(
        read_err.contains("channel read is ambiguous")
            && read_err.contains("tenex-edge channel read --channel"),
        "unexpected chat read error: {read_err}"
    );

    stop_daemon(&home);
}

/// A chat message with NO `@mention` (no p-tag) must NOT route to any session's
/// inbox — it stays in relay_events as ambient context only, never ringing the
/// doorbell. Guards the p-tag-gate behaviour introduced alongside the first-turn
/// compact-notice feature.
#[test]
fn non_mention_chat_does_not_route_to_inbox() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();

    let (sender_canon, receiver_canon) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let s = c
            .call(
                "session_start",
                serde_json::json!({
                    "agent": "ambient-sender",
                    "session_id": "ambient-sender-sess",
                    "cwd": "/tmp"
                }),
            )
            .await
            .unwrap();
        let r = c
            .call(
                "session_start",
                serde_json::json!({
                    "agent": "ambient-receiver",
                    "session_id": "ambient-receiver-sess",
                    "cwd": "/tmp"
                }),
            )
            .await
            .unwrap();
        (
            s["session_id"].as_str().unwrap().to_string(),
            r["session_id"].as_str().unwrap().to_string(),
        )
    });

    // Write a plain channel message — no @mention in the body.
    let body = "no-mention ambient message for routing test";
    let out = run_cli_stdin_with_env_in_dir(
        &home,
        &["channel", "send"],
        &format!("{body}\n"),
        &[("TENEX_EDGE_AGENT", "ambient-sender")],
        std::path::Path::new("/tmp"),
    );
    assert!(
        out.status.success(),
        "chat write failed: {}",
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

    // Inbox for the receiver must be empty — no doorbell should ring.
    assert!(
        store
            .peek_pending_for_session(&receiver_canon)
            .unwrap()
            .is_empty(),
        "non-mention message must not route to receiver inbox"
    );
    // Sender never receives its own message either.
    assert!(
        store
            .peek_pending_for_session(&sender_canon)
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
