use serde_json::Value;
use std::path::Path;

/// Return why a hook should fail open before agent/PID discovery or RPC work.
pub(super) fn inapplicable(cwd: &Path) -> Option<(&'static str, Value)> {
    // A harness outside a registered workspace should proceed without fabric
    // features; an explicit command will surface the missing registration.
    if crate::daemon::workspace_path::channel_for_path(cwd).is_err() {
        return Some((
            "no-channel",
            serde_json::json!({ "cwd": cwd.to_string_lossy() }),
        ));
    }
    // Socket absence is authoritative for the fast no-spawn path. A stale
    // socket still falls through to the bounded RPC attempt.
    (!crate::daemon::socket_path().exists()).then(|| ("daemon-unavailable", serde_json::json!({})))
}
