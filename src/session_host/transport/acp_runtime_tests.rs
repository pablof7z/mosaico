use super::*;
use serde_json::json;

#[test]
fn acp_agent_message_chunk_text_is_captured() {
    let params = json!({
        "sessionId": "ses_1",
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hello " }
        }
    });
    assert_eq!(
        extract_assistant_text("session/update", &params).as_deref(),
        Some("hello ")
    );
}

#[test]
fn tool_call_updates_are_ignored() {
    let params = json!({
        "update": { "sessionUpdate": "tool_call", "toolCallId": "tc_1" }
    });
    assert_eq!(extract_assistant_text("session/update", &params), None);
}

#[test]
fn non_update_methods_yield_no_text() {
    let params = json!({ "content": { "type": "text", "text": "x" } });
    assert_eq!(extract_assistant_text("initialize", &params), None);
}

#[test]
fn app_server_message_delta_is_captured() {
    let params = json!({ "role": "assistant", "delta": { "text": "world" } });
    assert_eq!(
        extract_assistant_text("turn/output", &params).as_deref(),
        Some("world")
    );
}

#[test]
fn turn_id_extracted_from_common_spellings() {
    assert_eq!(
        extract_turn_id(&json!({ "turnId": "t1" })).as_deref(),
        Some("t1")
    );
    assert_eq!(
        extract_turn_id(&json!({ "turn_id": "t2" })).as_deref(),
        Some("t2")
    );
    assert_eq!(
        extract_turn_id(&json!({ "turn": { "id": "t3" } })).as_deref(),
        Some("t3")
    );
    assert_eq!(extract_turn_id(&json!({ "other": 1 })), None);
}

#[test]
fn runtime_tracks_turn_lifecycle_and_transcript() {
    let mut rt = AcpRuntime::default();
    // A turn starts and streams text.
    rt.note_update("turn/started", &json!({ "turnId": "t9" }));
    rt.note_update(
        "session/update",
        &json!({ "update": { "sessionUpdate": "agent_message_chunk",
                             "content": { "type": "text", "text": "abc" } } }),
    );
    assert_eq!(rt.steer_state(), SteerState::Ready("t9".into()));
    assert_eq!(rt.transcript(), "abc");
    // The turn ends: no longer steerable.
    rt.note_update("turn/completed", &json!({ "turnId": "t9" }));
    assert_eq!(rt.steer_state(), SteerState::Idle);
}

#[test]
fn mark_helpers_flip_active_flag() {
    let mut rt = AcpRuntime::default();
    rt.mark_turn_started();
    // Active but no id yet -> gate the steer until the id is known (defect #2):
    // must NOT read as Idle, which would start a second concurrent turn.
    assert_eq!(rt.steer_state(), SteerState::AwaitingId);
    rt.note_update("x/update", &json!({ "turnId": "z" }));
    assert_eq!(rt.steer_state(), SteerState::Ready("z".into()));
    rt.mark_turn_finished();
    assert_eq!(rt.steer_state(), SteerState::Idle);
}
