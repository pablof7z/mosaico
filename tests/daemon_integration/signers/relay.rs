use serde_json::Value;
use std::process::Command;

/// The `name` field of the newest kind:0 profile authored by `pubkey`, if any.
/// Used to prove each agent instance publishes its OWN display label (issue #98):
/// "claude1", "claude2", etc. never clobber each other.
pub(super) fn kind0_name_for_author(relay: &str, pubkey: &str) -> Option<String> {
    let output = Command::new("nak")
        .args(["req", "-k", "0", "-a", pubkey, "-l", "5", relay])
        .output()
        .expect("run nak req kind:0");
    if !output.status.success() {
        return None;
    }
    // A relay may hold more than one replaceable copy; trust the newest.
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event.get("pubkey").and_then(Value::as_str) == Some(pubkey))
        .max_by_key(|event| event.get("created_at").and_then(Value::as_i64).unwrap_or(0))
        .and_then(|event| {
            let content = event.get("content")?.as_str()?.to_string();
            let meta: Value = serde_json::from_str(&content).ok()?;
            meta.get("name")?.as_str().map(str::to_string)
        })
}

pub(super) fn event_author(relay: &str, event_id: &str) -> Option<String> {
    let output = Command::new("nak")
        .args(["req", "-i", event_id, "-l", "1", relay])
        .output()
        .expect("run nak req by event id");
    output.status.success().then_some(())?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|event| event.get("pubkey")?.as_str().map(str::to_string))
}
