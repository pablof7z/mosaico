use super::*;
use serde_json::json;

#[test]
fn turn_id_extracted_from_current_notification_shapes() {
    assert_eq!(
        extract_turn_id(&json!({ "turnId": "t1" })).as_deref(),
        Some("t1")
    );
    assert_eq!(
        extract_turn_id(&json!({ "turn": { "id": "t2" } })).as_deref(),
        Some("t2")
    );
    assert_eq!(extract_turn_id(&json!({ "other": 1 })), None);
}

#[test]
fn runtime_tracks_turn_lifecycle() {
    let mut rt = AcpRuntime::default();
    // A turn starts and may stream updates without changing its identity.
    rt.note_update("turn/started", &json!({ "turn": { "id": "t9" } }));
    rt.note_update(
        "session/update",
        &json!({ "update": { "sessionUpdate": "agent_message_chunk",
                             "content": { "type": "text", "text": "abc" } } }),
    );
    assert_eq!(rt.steer_state(), SteerState::Ready("t9".into()));
    // The turn ends: no longer steerable.
    rt.note_update("turn/completed", &json!({ "turn": { "id": "t9" } }));
    assert_eq!(rt.steer_state(), SteerState::Idle);
}

#[test]
fn mark_helpers_flip_active_flag() {
    let mut rt = AcpRuntime::default();
    rt.mark_turn_started();
    // Active but no id yet -> gate the steer until the id is known (defect #2):
    // must NOT read as Idle, which would start a second concurrent turn.
    assert_eq!(rt.steer_state(), SteerState::AwaitingId);
    rt.note_update("item/started", &json!({ "turnId": "z" }));
    assert_eq!(rt.steer_state(), SteerState::Ready("z".into()));
    rt.mark_turn_finished();
    assert_eq!(rt.steer_state(), SteerState::Idle);
}
