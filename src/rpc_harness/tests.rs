use super::protocol::{classify, Inbound, StopReason};
use super::*;

#[test]
fn classify_response_result() {
    let v = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": { "sessionId": "ses_x" } });
    match classify(v) {
        Inbound::Response { id, result } => {
            assert_eq!(id, 1);
            assert_eq!(result.unwrap()["sessionId"], "ses_x");
        }
        _ => panic!("expected response"),
    }
}

#[test]
fn classify_response_error() {
    let v = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32000, "message": "boom" } });
    match classify(v) {
        Inbound::Response { id, result } => {
            assert_eq!(id, 2);
            let e = result.unwrap_err();
            assert_eq!(e.code, -32000);
            assert_eq!(e.message, "boom");
        }
        _ => panic!("expected error response"),
    }
}

#[test]
fn classify_agent_request() {
    let v = serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "session/request_permission",
        "params": { "options": [{ "optionId": "allow-once", "kind": "allow_once" }] }
    });
    match classify(v) {
        Inbound::Request { method, params, .. } => {
            assert_eq!(method, "session/request_permission");
            assert!(params.get("options").is_some());
        }
        _ => panic!("expected request"),
    }
}

#[test]
fn classify_notification() {
    let v = serde_json::json!({ "jsonrpc": "2.0", "method": "session/update", "params": {} });
    match classify(v) {
        Inbound::Notification { method, .. } => assert_eq!(method, "session/update"),
        _ => panic!("expected notification"),
    }
}

#[test]
fn permission_allow_all_prefers_allow_kind() {
    let policy = PermissionPolicy::AllowAll;
    let params = serde_json::json!({
        "options": [
            { "optionId": "reject", "kind": "reject" },
            { "optionId": "yes", "kind": "allow_once" }
        ]
    });
    assert_eq!(policy.choose(&params).as_deref(), Some("yes"));
}

#[test]
fn permission_allow_all_falls_back_to_first() {
    let policy = PermissionPolicy::AllowAll;
    let params = serde_json::json!({ "options": [{ "optionId": "only" }] });
    assert_eq!(policy.choose(&params).as_deref(), Some("only"));
}

#[test]
fn stop_reason_mapping() {
    assert_eq!(StopReason::from_wire("end_turn"), StopReason::EndTurn);
    assert_eq!(StopReason::from_wire("cancelled"), StopReason::Cancelled);
    assert_eq!(StopReason::from_wire("weird"), StopReason::Other);
}

#[tokio::test]
async fn fs_bridge_jails_writes() {
    let dir = std::env::temp_dir().join(format!("rpc-fs-jail-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let fs = FsBridge { root: dir.clone() };
    // Escape attempt is refused.
    let esc = fs
        .write_text(&serde_json::json!({ "path": "/etc/should_not_write", "content": "x" }))
        .await;
    assert!(esc.is_err());
    // In-jail write + read round-trips.
    let w = fs
        .write_text(&serde_json::json!({ "path": "note.txt", "content": "hello" }))
        .await;
    assert!(w.is_ok(), "in-jail write should succeed: {w:?}");
    let r = fs
        .read_text(&serde_json::json!({ "path": "note.txt" }))
        .await
        .unwrap();
    assert_eq!(r["content"], "hello");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn rpc_kill_terminates_the_owned_process_group() {
    let cwd = tempfile::tempdir().unwrap();
    let descendant_pid = cwd.path().join("descendant.pid");
    let cfg = SpawnConfig {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "/bin/sleep 60 & echo $! > \"$1\"; wait".into(),
            "mosaico-rpc-test".into(),
            descendant_pid.to_string_lossy().into_owned(),
        ],
        cwd: cwd.path().to_path_buf(),
        env: vec![],
        env_remove: vec![],
        dialect: Dialect::Acp,
        callbacks: Callbacks::allow_all(cwd.path().to_path_buf()),
    };
    let (handle, _updates) = RpcHandle::spawn(cfg).await.unwrap();
    for _ in 0..100 {
        if descendant_pid.is_file() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let descendant = std::fs::read_to_string(&descendant_pid)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert!(crate::liveness::pid_alive(descendant));

    handle.kill().await.unwrap();

    assert!(!crate::liveness::pid_alive(descendant));
}

async fn silent_app_server_handle() -> RpcHandle {
    let cwd = std::env::temp_dir();
    let cfg = SpawnConfig {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "IFS= read -r line; while IFS= read -r line; do :; done".into(),
        ],
        cwd: cwd.clone(),
        env: vec![],
        env_remove: vec![],
        dialect: Dialect::AppServer,
        callbacks: Callbacks::allow_all(cwd),
    };
    RpcHandle::spawn(cfg).await.unwrap().0
}

#[tokio::test]
async fn turn_observer_is_unique_and_drop_releases_its_route() {
    let handle = silent_app_server_handle().await;
    let observer = handle.register_turn_waiter("thread-1").unwrap();
    assert!(handle.register_turn_waiter("thread-1").is_err());
    drop(observer);
    let replacement = handle.register_turn_waiter("thread-1").unwrap();
    drop(replacement);
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn request_timeout_removes_its_correlation_entry() {
    let handle = silent_app_server_handle().await;
    let error = handle
        .request_timeout(
            "thread/read",
            serde_json::json!({"threadId":"thread-1","includeTurns":true}),
            std::time::Duration::from_millis(10),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, RpcError::Timeout));
    assert_eq!(handle.pending_request_count(), 0);
    handle.kill().await.unwrap();
}

/// LIVE smoke against real `opencode acp` on this machine. Gated so CI without
/// auth skips it. Run with:
///   MOSAICO_RPC_LIVE=1 cargo test --lib -- --ignored rpc_harness::tests::live_opencode
#[tokio::test]
#[ignore]
async fn live_opencode_roundtrip_and_resume() {
    if std::env::var("MOSAICO_RPC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live test (set MOSAICO_RPC_LIVE=1)");
        return;
    }
    let cwd = std::env::temp_dir().join(format!("rpc-live-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();

    let cfg = SpawnConfig {
        program: "opencode".into(),
        args: vec!["acp".into()],
        cwd: cwd.clone(),
        env: vec![],
        env_remove: vec![],
        dialect: Dialect::Acp,
        callbacks: Callbacks::allow_all(cwd.clone()),
    };
    let (handle, mut updates) = RpcHandle::spawn(cfg).await.expect("spawn opencode acp");
    let client = AcpClient::new(handle.clone());

    client.initialize().await.expect("initialize");
    let session_id = client.session_new(&cwd, None).await.expect("session/new");
    eprintln!("session_id = {session_id}");

    // Drain updates in the background to prove chunks arrive.
    let got_chunk = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let gc = got_chunk.clone();
    tokio::spawn(async move {
        while let Some(u) = updates.recv().await {
            if u.method.contains("update") {
                gc.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });

    let stop = client
        .session_prompt(&session_id, "Reply with exactly one word: PONG")
        .await
        .expect("session/prompt");
    assert_eq!(stop, StopReason::EndTurn, "expected end_turn");

    handle.kill().await.unwrap();

    // Fresh process: session/load proves cross-process resume.
    let cfg2 = SpawnConfig {
        program: "opencode".into(),
        args: vec!["acp".into()],
        cwd: cwd.clone(),
        env: vec![],
        env_remove: vec![],
        dialect: Dialect::Acp,
        callbacks: Callbacks::allow_all(cwd.clone()),
    };
    let (handle2, _u2) = RpcHandle::spawn(cfg2).await.expect("spawn opencode acp #2");
    let client2 = AcpClient::new(handle2.clone());
    client2.initialize().await.expect("initialize #2");
    client2
        .session_load(&session_id, &cwd)
        .await
        .expect("session/load cross-process");
    eprintln!(
        "resume ok; got_chunk={}",
        got_chunk.load(std::sync::atomic::Ordering::Relaxed)
    );
    handle2.kill().await.unwrap();
}
