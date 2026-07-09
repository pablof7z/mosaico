use super::*;
use serde_json::json;

#[test]
fn validate_render_lists_identity_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "profile:pk-agent",
        "verdict": "passed",
        "ok": true,
        "checks": [
            {"name":"identity","status":"passed","summary":"profile `pk-agent` resolves to `pk-agent` with profile and local identity"}
        ],
        "identity_evidence": {
            "kind": "profile",
            "requested": "pk-agent",
            "resolved_pubkey": "pk-agent",
            "found": true,
            "profile_found": true,
            "profile_name": "Coder",
            "profile_slug": "coder",
            "profile_host": "macos",
            "profile_is_backend": false,
            "profile_updated_at": 100,
            "identity_found": true,
            "identity": {
                "codename": "willow-echo-042",
                "alive": true,
                "session_id": "s1",
                "channel_h": "room",
                "native_id": "native-s1"
            },
            "derived_identity_count": 1,
            "bound_session_found": true,
            "bound_session_id": "s1",
            "bound_session_alive": true,
            "bound_session_channel": "room",
            "member_channel_count": 1,
            "admin_channel_count": 0,
            "reason": ""
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("identity evidence"));
    assert!(text.contains("profile=pk-agent -> pk-agent profile=true identity=true"));
    assert!(text.contains("identity codename=willow-echo-042 alive=true session=s1"));
    assert!(text.contains("memberships=1 admin_channels=0"));
}
