use super::*;
use serde_json::json;

#[test]
fn validate_render_lists_workspace_evidence() {
    let v = json!({
        "verb": "validate",
        "target": "channel:room",
        "verdict": "passed",
        "ok": true,
        "checks": [
            {"name":"workspace","status":"passed","summary":"channel `room` root path `/tmp/room` exists"}
        ],
        "workspace_evidence": {
            "channel_h": "room",
            "root_channel": "root",
            "channel_found": true,
            "found": true,
            "direct_binding_found": false,
            "inherited_binding": true,
            "binding_channel_h": "root",
            "abs_path": "/tmp/root",
            "path_absolute": true,
            "path_exists": true,
            "path_is_dir": true,
            "updated_at": 100
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("workspace evidence"));
    assert!(text.contains("channel=room root_channel=root"));
    assert!(text.contains("binding_channel=root path=/tmp/root"));
    assert!(text.contains("direct=false inherited=true"));
}

#[test]
fn validate_render_lists_workspace_reason() {
    let v = json!({
        "verb": "validate",
        "target": "channel:missing",
        "verdict": "passed_with_limitations",
        "ok": true,
        "checks": [
            {"name":"workspace","status":"not_proven","summary":"channel `missing` has no local workspace binding"}
        ],
        "limitations": ["no workspace_roots row exists for this channel or its root channel"],
        "workspace_evidence": {
            "channel_h": "missing",
            "root_channel": "",
            "channel_found": false,
            "found": false,
            "direct_binding_found": false,
            "inherited_binding": false,
            "binding_channel_h": "",
            "reason": "no workspace_roots row exists for this channel or its root channel"
        }
    });

    let text = render_validate(&v);

    assert!(text.contains("workspace evidence"));
    assert!(text.contains("no workspace_roots row exists"));
}
