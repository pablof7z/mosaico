use crate::daemon_harness::*;
use std::time::Duration;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;
use tenex_edge::util::session_codename;

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

    // Mention is now inline in the body as `@<codename>` — no --mention flag.
    let receiver_codename = session_codename(&receiver_canon);
    let body = format!("hello @{receiver_codename} from redirected stdin");
    let out = run_cli_stdin(
        &home,
        &["chat", "write", "--session", "chat-sender-session"],
        &format!("{body}\n"),
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
        stdout.contains(&format!("<chat-sender@test-host> {body} [")),
        "chat read should render sender, host, body, and timestamp; got: {stdout}"
    );

    let mut received = false;
    for _ in 0..12 {
        let store = Store::open(&home.store_path()).unwrap();
        let rows = store.peek_chat(&receiver_canon).unwrap();
        if let Some(row) = rows.iter().find(|row| row.body == body) {
            assert_eq!(row.mentioned_session, receiver_canon);
            assert_eq!(row.from_session, sender_canon);
            received = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(received, "receiver did not get live chat row");

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
        assert!(
            pending
                .iter()
                .any(|row| { row["from_slug"] == "chat-sender" && row["body"] == body }),
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
            recent
                .iter()
                .any(|row| { row["from_slug"] == "chat-sender" && row["body"] == body }),
            "statusline should briefly linger drained chat mentions: {statusline}"
        );
    });

    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        store.peek_chat(&sender_canon).unwrap().is_empty(),
        "sender should not receive its own chat row"
    );

    stop_daemon(&home);
}
