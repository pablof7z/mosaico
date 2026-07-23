use super::*;
use crate::rpc_harness::{Callbacks, Dialect, SpawnConfig};

#[path = "tests/protocol.rs"]
mod protocol;

fn fixture(script: &str) -> SpawnConfig {
    let cwd = std::env::temp_dir();
    SpawnConfig {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: cwd.clone(),
        env: Vec::new(),
        env_remove: Vec::new(),
        dialect: Dialect::AppServer,
        callbacks: Callbacks::allow_all(cwd),
    }
}

#[test]
fn custom_agent_uses_native_thread_start_fields() {
    let cwd = std::path::Path::new("/workspace");
    let config = serde_json::json!({
        "model": "gpt-5.4",
        "model_reasoning_effort": "high"
    });

    assert_eq!(
        thread_start_params(cwd, Some("Review carefully"), Some(&config)),
        serde_json::json!({
            "cwd": "/workspace",
            "developerInstructions": "Review carefully",
            "config": {
                "model": "gpt-5.4",
                "model_reasoning_effort": "high"
            }
        })
    );
}

#[test]
fn default_thread_start_omits_agent_fields() {
    assert_eq!(
        thread_start_params(std::path::Path::new("/workspace"), None, None),
        serde_json::json!({ "cwd": "/workspace" })
    );
}

#[tokio::test]
async fn lost_terminal_notification_reconciles_from_thread_read() {
    let script = r#"
IFS= read -r baseline || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[]}}}'
IFS= read -r start || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"turn":{"id":"turn-1","items":[],"status":"inProgress"}}}'
while IFS= read -r read_turn; do
  id=$(printf '%s' "$read_turn" | /usr/bin/sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  printf '{"jsonrpc":"2.0","id":%s,"result":{"thread":{"id":"thread-1","turns":[{"id":"turn-1","items":[],"status":"completed"}]}}}\n' "$id"
done
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        AppServerClient::new(handle.clone()).turn_start("thread-1", "work"),
    )
    .await
    .expect("reconciliation did not run")
    .unwrap();
    assert_eq!(
        outcome,
        TurnOutcome::Completed {
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into()
        }
    );
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn fresh_thread_uses_exact_start_history_without_a_preflight_read() {
    let script = r#"
IFS= read -r start_thread || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[]}}}'
IFS= read -r start_turn || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"turn":{"id":"turn-1","items":[],"status":"inProgress"}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","items":[],"status":"completed"}}}'
while IFS= read -r line; do :; done
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let client = AppServerClient::new(handle.clone());
    let thread_id = client
        .thread_start(std::path::Path::new("/workspace"), None, None)
        .await
        .unwrap();
    let outcome = client.turn_start(&thread_id, "work").await.unwrap();
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn resumed_thread_uses_exact_resume_history_without_a_preflight_read() {
    let script = r#"
IFS= read -r resume || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[{"id":"old-turn","items":[],"status":"completed"}]}}}'
IFS= read -r start_turn || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"turn":{"id":"turn-1","items":[],"status":"inProgress"}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","items":[],"status":"completed"}}}'
while IFS= read -r line; do :; done
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let client = AppServerClient::new(handle.clone());
    client
        .thread_resume("thread-1", std::path::Path::new("/workspace"))
        .await
        .unwrap();
    let outcome = client.turn_start("thread-1", "work").await.unwrap();
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn in_progress_terminal_notification_is_reconciled_as_failure() {
    let script = r#"
IFS= read -r baseline || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[]}}}'
IFS= read -r start || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"turn":{"id":"turn-1","items":[],"status":"inProgress"}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","items":[],"status":"inProgress"}}}'
while IFS= read -r read_turn; do
  id=$(printf '%s' "$read_turn" | /usr/bin/sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  printf '{"jsonrpc":"2.0","id":%s,"result":{"thread":{"id":"thread-1","turns":[{"id":"turn-1","items":[],"status":"failed","error":{"message":"model rejected"}}]}}}\n' "$id"
done
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let outcome = AppServerClient::new(handle.clone())
        .turn_start("thread-1", "work")
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        TurnOutcome::Failed {
            error: Some(TurnFailure { ref message, .. }),
            ..
        } if message == "model rejected"
    ));
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn thread_status_change_triggers_exact_turn_reconciliation() {
    let script = r#"
IFS= read -r baseline || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[]}}}'
IFS= read -r start || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"turn":{"id":"turn-1","items":[],"status":"inProgress"}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"thread/status/changed","params":{"threadId":"thread-1","status":{"type":"idle"}}}'
while IFS= read -r read_turn; do
  id=$(printf '%s' "$read_turn" | /usr/bin/sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  printf '{"jsonrpc":"2.0","id":%s,"result":{"thread":{"id":"thread-1","turns":[{"id":"turn-1","items":[],"status":"interrupted"}]}}}\n' "$id"
done
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let outcome = AppServerClient::new(handle.clone())
        .turn_start("thread-1", "work")
        .await
        .unwrap();
    assert!(matches!(outcome, TurnOutcome::Interrupted { .. }));
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn child_exit_ends_the_waiter_without_a_wall_clock_completion() {
    let script = r#"
IFS= read -r baseline || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[]}}}'
IFS= read -r start || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"turn":{"id":"turn-1","items":[],"status":"inProgress"}}}'
exit 0
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let error = AppServerClient::new(handle)
        .turn_start("thread-1", "work")
        .await
        .unwrap_err();
    assert!(matches!(error, RpcError::ChildExited));
}

#[tokio::test]
async fn lost_turn_start_response_recovers_without_replay() {
    let script = r#"
IFS= read -r baseline || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[]}}}'
IFS= read -r start || exit 1
printf '%s\n' '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","items":[],"status":"completed"}}}'
while IFS= read -r line; do :; done
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let outcome = AppServerClient::new(handle.clone())
        .turn_start("thread-1", "work")
        .await
        .unwrap();
    assert_eq!(
        outcome,
        TurnOutcome::Completed {
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into()
        }
    );
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn uncertain_start_recovers_exact_new_turn_from_thread_read() {
    let script = r#"
IFS= read -r baseline || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[{"id":"old-turn","items":[],"status":"completed"}]}}}'
IFS= read -r start || exit 1
IFS= read -r read_turn || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"thread":{"id":"thread-1","turns":[{"id":"old-turn","items":[],"status":"completed"},{"id":"turn-1","items":[],"status":"completed"}]}}}'
while IFS= read -r line; do :; done
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let outcome = AppServerClient::new(handle.clone())
        .turn_start("thread-1", "work")
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        TurnOutcome::Completed { ref turn_id, .. } if turn_id == "turn-1"
    ));
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn explicit_kill_ends_turn_wait_deterministically() {
    let script = r#"
IFS= read -r baseline || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-1","turns":[]}}}'
IFS= read -r start || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"turn":{"id":"turn-1","items":[],"status":"inProgress"}}}'
while IFS= read -r line; do :; done
"#;
    let (handle, _updates) = RpcHandle::spawn(fixture(script)).await.unwrap();
    let client = AppServerClient::new(handle.clone());
    let turn = tokio::spawn(async move { client.turn_start("thread-1", "work").await });
    tokio::task::yield_now().await;
    handle.kill().await.unwrap();
    let result = turn.await.unwrap();
    assert!(
        matches!(result, Err(RpcError::ChildExited)),
        "unexpected cancellation result: {result:?}"
    );
}
