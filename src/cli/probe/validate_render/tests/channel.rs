use super::*;
use serde_json::json;

#[test]
fn validate_render_lists_channel_readiness_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "readiness:room",
        "verdict": "failed",
        "ok": false,
        "checks": [
            {"name":"channel_readiness","status":"failed","summary":"1 channel_ready failure(s) recorded for this channel"}
        ],
        "limitations": ["relay rejected event: timeout"],
        "channel_evidence": {
            "kind": "readiness",
            "channel_h": "room",
            "found": false,
            "readiness_ok": false,
            "session_start_count": 1,
            "session_start_channel_ready_count": 1,
            "session_start_failed_count": 1,
            "channel_ready_failure_count": 1,
            "provider_attempt_count": 0,
            "provider_degraded_count": 0,
            "readiness_summary": "1 channel_ready failure(s) recorded for this channel",
            "readiness_reason": "relay rejected event: timeout",
            "session_start_rows": [{
                "session_id": "s1",
                "action": "RecordFailed",
                "channel_h": "room",
                "has_channel_ready_intent": true,
                "has_spawn_intent": true,
                "ensure_subscription": true,
                "reassert": false,
                "failure_stage": "channel_ready",
                "failure_error": "relay rejected event: timeout"
            }]
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("channel evidence"));
    assert!(text.contains("room: not materialized"));
    assert!(text.contains(
        "readiness ok=false attempts=1 channel_ready=1 failures=1 provider_attempts=0 provider_degraded=0"
    ));
    assert!(text.contains("session_start/s1 action=RecordFailed channel_ready=true spawn=true"));
    assert!(text.contains("failed at channel_ready: relay rejected event: timeout"));
}

#[test]
fn validate_render_lists_provider_readiness_attempts() {
    let v = json!({
        "verb": "validate",
        "target": "readiness:room",
        "verdict": "failed",
        "ok": false,
        "checks": [
            {"name":"channel_readiness","status":"failed","summary":"1 provider readiness attempt(s) degraded for this channel"}
        ],
        "limitations": ["management key is not admin and self-grant failed"],
        "channel_evidence": {
            "kind": "readiness",
            "channel_h": "room",
            "found": false,
            "readiness_ok": false,
            "session_start_count": 0,
            "session_start_channel_ready_count": 0,
            "session_start_failed_count": 0,
            "channel_ready_failure_count": 0,
            "provider_attempt_count": 1,
            "provider_degraded_count": 1,
            "readiness_summary": "1 provider readiness attempt(s) degraded for this channel",
            "readiness_reason": "management key is not admin and self-grant failed",
            "provider_attempt_rows": [{
                "id": 7,
                "channel_h": "room",
                "expect_member": "pk-member",
                "parent_hint": "root",
                "source": "provider.ensure_channel_ready",
                "outcome": "degraded",
                "reason": "management key is not admin and self-grant failed",
                "created_at": 100
            }]
        }
    });

    let text = render_validate(&v);

    assert!(text.contains(
        "readiness ok=false attempts=0 channel_ready=0 failures=0 provider_attempts=1 provider_degraded=1"
    ));
    assert!(text.contains("provider_attempt/7 outcome=degraded member=pk-member"));
    assert!(text.contains("reason: management key is not admin and self-grant failed"));
}

#[test]
fn validate_render_lists_provider_attempts_for_plain_channel() {
    let v = json!({
        "verb": "validate",
        "target": "channel:room",
        "verdict": "passed_with_limitations",
        "ok": true,
        "checks": [
            {"name":"channel","status":"passed","summary":"channel `room` is materialized from relay kind:39000"}
        ],
        "limitations": ["provider readiness attempts are recorded; inspect provider_attempt:<id> for the provisioning trace"],
        "channel_evidence": {
            "kind": "channel",
            "channel_h": "room",
            "found": true,
            "human_name": "Room",
            "parent": "",
            "root_channel": "room",
            "member_count": 2,
            "admin_count": 1,
            "membership_snapshot": true,
            "readiness_ok": true,
            "session_start_count": 0,
            "session_start_channel_ready_count": 0,
            "session_start_failed_count": 0,
            "channel_ready_failure_count": 0,
            "provider_attempt_count": 1,
            "provider_degraded_count": 0,
            "readiness_summary": "relay metadata and membership snapshots are hydrated",
            "provider_attempt_rows": [{
                "id": 7,
                "channel_h": "room",
                "expect_member": "pk-member",
                "parent_hint": "",
                "source": "provider.ensure_channel_ready",
                "outcome": "ready",
                "reason": "channel readiness verified",
                "created_at": 100
            }],
            "reason": "provider readiness attempts are recorded; inspect provider_attempt:<id> for the provisioning trace"
        }
    });

    let text = render_validate(&v);

    assert!(text.contains(
        "readiness ok=true attempts=0 channel_ready=0 failures=0 provider_attempts=1 provider_degraded=0"
    ));
    assert!(text.contains("provider_attempt/7 outcome=ready member=pk-member"));
    assert!(text.contains("reason: channel readiness verified"));
}
