use serde_json::Value;
use std::process::Command;

/// The `name` field of the newest kind:0 profile authored by `pubkey`, if any.
/// Used to prove each agent instance publishes its OWN display label (issue #98):
/// "claude1", "claude2", etc. never clobber each other.
pub(super) fn kind0_name_for_author(relay: &str, pubkey: &str) -> Option<String> {
    let output = Command::new(crate::common::nak_bin())
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

/// The author of the one relay-stored chat whose id starts with the short id
/// the daemon returned for it.
pub(super) fn chat_author_by_id_prefix(relay: &str, id_prefix: &str) -> Option<String> {
    let output = Command::new(crate::common::nak_bin())
        .args(["req", "-k", "9", "-l", "500", relay])
        .output()
        .expect("run nak req for chats");
    output.status.success().then_some(())?;
    let mut authors = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| {
            event
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.starts_with(id_prefix))
        })
        .filter_map(|event| event.get("pubkey")?.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    (authors.len() == 1).then(|| authors.remove(0))
}
