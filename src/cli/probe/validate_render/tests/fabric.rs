use super::*;
use serde_json::json;

#[test]
fn validate_render_lists_channel_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "channel:room",
        "verdict": "passed_with_limitations",
        "ok": true,
        "checks": [
            {"name":"channel","status":"passed","summary":"channel `room` is materialized from relay kind:39000"}
        ],
        "limitations": ["channel provisioning is a host/provider side effect"],
        "channel_evidence": {
            "target": "channel:room",
            "channel_h": "room",
            "kind": "channel",
            "supported": true,
            "found": true,
            "human_name": "Room",
            "parent": "",
            "root_channel": "room",
            "member_count": 2,
            "admin_count": 1,
            "membership_snapshot": true,
            "reason": "channel provisioning is a host/provider side effect"
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("channel evidence"));
    assert!(text.contains("room: materialized"));
    assert!(text.contains("members=2 admins=1 membership_snapshot=true"));
    assert!(text.contains("host/provider side effect"));
}

#[test]
fn validate_render_lists_awareness_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "awareness:room",
        "verdict": "passed",
        "ok": true,
        "checks": [
            {"name":"awareness","status":"passed","summary":"awareness for channel `room` has 2 live row(s)"}
        ],
        "limitations": [],
        "awareness_evidence": {
            "channel_h": "room",
            "channel_confirmed": true,
            "channel_name": "Room",
            "parent": "",
            "root_channel": "room",
            "membership_snapshot": true,
            "member_count": 3,
            "admin_count": 1,
            "row_count": 2,
            "local_row_count": 1,
            "peer_row_count": 1,
            "fresh_row_count": 2,
            "spawnable_count": 4
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("awareness evidence"));
    assert!(text.contains("room: confirmed"));
    assert!(text.contains("members=3 admins=1 membership_snapshot=true"));
    assert!(text.contains("live_rows=2 local=1 peer=1 fresh=2 spawnable_local=4"));
}

#[test]
fn validate_render_lists_message_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "message:event-123",
        "verdict": "passed_with_limitations",
        "ok": true,
        "checks": [
            {"name":"message","status":"passed","summary":"message `event-123` is accepted in channel `room`"}
        ],
        "limitations": ["message validation proves the local canonical chat read model"],
        "message_evidence": {
            "requested_id": "event-123",
            "message_id": "event-123",
            "found": true,
            "channel_h": "room",
            "channel_confirmed": true,
            "direction": "outbound",
            "sync_state": "accepted",
            "author_session": "sender-session",
            "native_event_id": "event-123",
            "recipient_count": 2,
            "delivered_recipient_count": 1,
            "pending_recipient_count": 1,
            "body_len": 23,
            "body_preview": "hello from the fabric",
            "reason": "message validation proves the local canonical chat read model"
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("message evidence"));
    assert!(text.contains("event-123: accepted channel=room (confirmed)"));
    assert!(text.contains("direction=outbound author_session=\"sender-session\""));
    assert!(text.contains("recipients=2 delivered=1 pending=1"));
    assert!(text.contains("preview=\"hello from the fabric\""));
}

#[test]
fn validate_render_lists_session_start_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "session_start:s1",
        "verdict": "failed",
        "ok": false,
        "checks": [
            {"name":"session_start_outcome","status":"failed","summary":"session_start `s1` failed at channel_ready"}
        ],
        "limitations": ["relay rejected event: timeout"],
        "session_start_evidence": {
            "session_id": "s1",
            "found": true,
            "action": "RecordFailed",
            "channel_h": "room",
            "reassert": false,
            "has_channel_ready_intent": true,
            "has_spawn_intent": true,
            "ensure_subscription": true,
            "replay_chat": false,
            "failure_stage": "channel_ready",
            "failure_error": "relay rejected event: timeout",
            "reason": "relay rejected event: timeout"
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("session start evidence"));
    assert!(text.contains("s1: RecordFailed channel=room reassert=false"));
    assert!(text.contains("planned host effects: channel_ready, spawn, subscription"));
    assert!(text.contains("failed at channel_ready: relay rejected event: timeout"));
}

#[test]
fn validate_render_lists_session_watch_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "watch:s1",
        "verdict": "passed",
        "ok": true,
        "checks": [
            {"name":"session_watch_outcome","status":"passed","summary":"session_watch `s1` is open and pid 42 is alive"}
        ],
        "limitations": [],
        "session_watch_evidence": {
            "session_id": "s1",
            "graph_open": true,
            "session_row_found": true,
            "session_alive": true,
            "channel_h": "room",
            "agent_slug": "coder",
            "child_pid": 42,
            "process_alive": true,
            "last_seen": 100,
            "reason": ""
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("session watch evidence"));
    assert!(text.contains("s1: graph=open session_row=alive"));
    assert!(text.contains("channel=room agent=coder last_seen=100"));
    assert!(text.contains("pid=42 process_alive=true"));
}
