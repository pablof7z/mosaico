use super::*;
use serde_json::json;

#[test]
fn validate_render_lists_recipient_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "recipient:event-123:pk-recipient",
        "verdict": "passed",
        "ok": true,
        "checks": [
            {"name":"recipient","status":"passed","summary":"message `event-123` was delivered to recipient `pk-recipient`"}
        ],
        "recipient_evidence": {
            "message_prefix": "event-123",
            "message_id": "event-123",
            "recipient_pubkey": "pk-recipient",
            "message_found": true,
            "found": true,
            "delivered": true,
            "pending": false,
            "message_channel_h": "room",
            "message_sync_state": "accepted",
            "message_native_event_id": "event-123",
            "matching_row_count": 1,
            "recipient_count": 1,
            "profile_found": true,
            "profile_slug": "agent",
            "reason": "message_recipients contains a delivered edge for this recipient"
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("recipient evidence"));
    assert!(text.contains("message=event-123 recipient=pk-recipient"));
    assert!(text.contains("rows=1 total_recipients=1"));
    assert!(text.contains("message_recipients contains a delivered edge"));
}
