use super::*;

#[test]
fn classifies_response_and_terminal_event() {
    let response = serde_json::json!({
        "id":"41", "type":"response", "command":"get_state", "success":true,
        "data":{"sessionId":"pi-session"}
    });
    match protocol::classify_for(Dialect::PiRpc, response) {
        Inbound::Response { id, result } => {
            assert_eq!(id, 41);
            assert_eq!(result.unwrap()["sessionId"], "pi-session");
        }
        _ => panic!("expected Pi response"),
    }

    let event = serde_json::json!({"type":"agent_settled"});
    match protocol::classify_for(Dialect::PiRpc, event) {
        Inbound::Notification { method, params } => {
            assert_eq!(method, "agent_settled");
            assert_eq!(params["type"], "agent_settled");
        }
        _ => panic!("expected Pi event"),
    }
}

async fn pi_child(
    script: String,
) -> (
    RpcHandle,
    tokio::sync::mpsc::UnboundedReceiver<SessionUpdate>,
) {
    let cwd = std::env::temp_dir();
    let config = SpawnConfig {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script],
        cwd: cwd.clone(),
        env: vec![],
        env_remove: vec![],
        dialect: Dialect::PiRpc,
        callbacks: Callbacks::allow_all(cwd),
    };
    RpcHandle::spawn(config).await.unwrap()
}

#[tokio::test]
async fn prompt_ignores_agent_end_and_completes_on_agent_settled() {
    let (handle, mut updates) = pi_child(
        r#"IFS= read -r line || exit 1
id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([0-9][0-9]*\)".*/\1/p')
printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
printf '%s\n' '{"type":"agent_end","messages":[]}'
IFS= read -r line || exit 1
id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([0-9][0-9]*\)".*/\1/p')
printf '{"id":"%s","type":"response","command":"get_state","success":true,"data":{"sessionId":"pi-session"}}\n' "$id"
printf '%s\n' '{"type":"agent_settled"}'
while IFS= read -r line; do :; done"#
            .into(),
    )
    .await;
    let prompt = tokio::spawn({
        let handle = handle.clone();
        async move { PiRpcClient::new(handle).prompt("work").await }
    });
    let agent_end = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let update = updates.recv().await.expect("Pi update stream closed");
            if update.method == "agent_end" {
                break;
            }
        }
    })
    .await;
    assert!(agent_end.is_ok(), "Pi never emitted agent_end");
    assert!(!prompt.is_finished(), "agent_end must not settle a Pi turn");
    handle
        .pi_request_timeout(
            serde_json::json!({"type":"get_state"}),
            std::time::Duration::from_secs(10),
        )
        .await
        .expect("failed to release agent_settled fixture");
    tokio::time::timeout(std::time::Duration::from_secs(10), prompt)
        .await
        .expect("agent_settled did not complete the Pi turn")
        .expect("Pi prompt task panicked")
        .expect("Pi prompt failed");
    handle.kill().await.unwrap();
}

#[tokio::test]
async fn strict_lf_framing_preserves_unicode_line_separators() {
    let text = "before\u{2028}middle\u{2029}after";
    let response = serde_json::json!({
        "id": "1",
        "type": "response",
        "command": "get_state",
        "success": true,
        "data": {"sessionId": "pi-session", "text": text}
    })
    .to_string();
    assert!(response.contains('\u{2028}') && response.contains('\u{2029}'));
    let script = format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' '{}'\nwhile IFS= read -r line; do :; done",
        response
    );
    let (handle, _updates) = pi_child(script).await;
    let result = handle
        .pi_request_timeout(
            serde_json::json!({"type":"get_state"}),
            std::time::Duration::from_secs(2),
        )
        .await
        .expect("strict JSONL response failed");
    assert_eq!(result["text"], text);
    handle.kill().await.unwrap();
}

#[test]
fn preserves_rejection_error() {
    let response = serde_json::json!({
        "id":"7", "type":"response", "command":"prompt", "success":false,
        "error":"agent is already streaming"
    });
    match protocol::classify_for(Dialect::PiRpc, response) {
        Inbound::Response { result, .. } => {
            assert_eq!(result.unwrap_err().message, "agent is already streaming");
        }
        _ => panic!("expected Pi rejection"),
    }
}
