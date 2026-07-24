use super::*;
use serde_json::json;

fn update(method: &str, params: serde_json::Value) -> SessionUpdate {
    SessionUpdate {
        method: method.to_string(),
        params,
    }
}

#[test]
fn decodes_acp_agent_message_chunk() {
    // The one shape verified by existing repo fixtures.
    let u = update(
        "session/update",
        json!({"update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"}}}),
    );
    assert_eq!(decode(&u), Some(DeployEvent::Agent("hi".into())));
}

#[test]
fn decodes_acp_thought_chunk() {
    let u = update(
        "session/update",
        json!({"update": {"sessionUpdate": "agent_thought_chunk", "content": {"text": "reasoning"}}}),
    );
    assert_eq!(decode(&u), Some(DeployEvent::Thought("reasoning".into())));
}

#[test]
fn tool_calls_become_generic_activity() {
    let u = update(
        "session/update",
        json!({"update": {"sessionUpdate": "tool_call", "title": "bash", "status": "in_progress"}}),
    );
    assert_eq!(
        decode(&u),
        Some(DeployEvent::Activity("bash [in_progress]".into()))
    );
}

#[test]
fn lifecycle_notifications_are_ignored() {
    assert_eq!(decode(&update("turn/completed", json!({}))), None);
    assert_eq!(decode(&update("thread/status/changed", json!({}))), None);
}

#[test]
fn app_server_item_prefers_meaningful_text() {
    let u = update(
        "item/started",
        json!({"item": {"command": "docker run croissant"}}),
    );
    assert_eq!(
        decode(&u),
        Some(DeployEvent::Activity("docker run croissant".into()))
    );
}

#[test]
fn app_server_item_without_text_falls_back_to_method() {
    let u = update("item/completed", json!({"item": {"id": "x"}}));
    assert_eq!(
        decode(&u),
        Some(DeployEvent::Activity("item/completed".into()))
    );
}

#[test]
fn unknown_acp_update_kinds_are_dropped() {
    let u = update(
        "session/update",
        json!({"update": {"sessionUpdate": "available_commands_update"}}),
    );
    assert_eq!(decode(&u), None);
}
