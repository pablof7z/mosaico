use crate::daemon_harness::*;
use std::time::Duration;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[path = "messaging/explicit_destination.rs"]
mod explicit_destination;
#[path = "messaging/inbox_rows.rs"]
mod inbox_rows;
#[path = "messaging/non_mention.rs"]
mod non_mention;
#[path = "messaging/target_wire.rs"]
mod target_wire;
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
            .call(
                "who",
                serde_json::json!({"all": true, "all_workspaces": true}),
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
            .call(
                "who",
                serde_json::json!({"all": true, "all_workspaces": true}),
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
fn channel_send_stdin_enqueues_live_channel_chat_for_receiver() {
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
    let receiver_handle = Store::open(&home.store_path())
        .unwrap()
        .session_identity_for_session(&receiver_canon)
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
            &receiver_row.agent_pubkey,
            &receiver_handle,
            &receiver_handle,
            "test-host",
            false,
            now,
        )
        .unwrap();
    let body = format!("hello @{receiver_handle} from redirected stdin");
    let read_body = target_wire::redirected_stdin_rendered_body(&receiver_handle);
    let wire_body =
        target_wire::redirected_stdin_body_for_session(&home, &receiver_canon, &receiver_row);
    let out = run_cli_stdin_with_env_in_dir(
        &home,
        &["channel", "send"],
        &format!("{body}\n"),
        &[("TENEX_EDGE_AGENT", "chat-sender")],
        std::path::Path::new("/tmp"),
    );
    assert!(
        out.status.success(),
        "channel send failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // `channel read` renders relay-materialized chat; the send above may not have
    // propagated to the readable store yet, so poll the read until the body
    // renders rather than asserting on a single racy read.
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
                &[("TENEX_EDGE_AGENT", "chat-sender")],
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
        .get_session(&sender_canon)
        .unwrap()
        .expect("sender session row")
        .agent_pubkey;
    assert!(
        wait_until(Duration::from_secs(2), || Store::open(&home.store_path())
            .map(|store| receiver_inbox_rows(&store, &receiver_canon)
                .iter()
                .any(|row| row.body == wire_body))
            .unwrap_or(false)),
        "receiver did not get live chat row"
    );
    let store = Store::open(&home.store_path()).unwrap();
    // The inbound routing ledger may still be pending, or may already be marked
    // injected when a live PTY endpoint is present in the integration process.
    let rows = receiver_inbox_rows(&store, &receiver_canon);
    let row = rows
        .iter()
        .find(|row| row.body == wire_body)
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
            pending.iter().any(|row| { row["body"] == wire_body }),
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
            recent.iter().any(|row| { row["body"] == wire_body }),
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
