use serde_json::Value;
use std::fmt::Write as _;

pub(super) fn render(out: &mut String, evidence: &Value) {
    let _ = writeln!(out);
    let _ = writeln!(out, "recipient evidence");
    if !bool_at(evidence, "message_found") {
        let _ = writeln!(
            out,
            "  - message {}: not found",
            str_at(evidence, "message_prefix")
        );
    } else {
        let _ = writeln!(
            out,
            "  - message={} recipient={} found={} delivered={} pending={}",
            str_at(evidence, "message_id"),
            str_at(evidence, "recipient_pubkey"),
            bool_at(evidence, "found"),
            bool_at(evidence, "delivered"),
            bool_at(evidence, "pending")
        );
        let _ = writeln!(
            out,
            "  - channel={} sync={} native_event_id={:?}",
            str_at(evidence, "message_channel_h"),
            str_at(evidence, "message_sync_state"),
            str_at(evidence, "message_native_event_id")
        );
        let _ = writeln!(
            out,
            "  - rows={} total_recipients={}",
            int_at(evidence, "matching_row_count"),
            int_at(evidence, "recipient_count")
        );
        let _ = writeln!(
            out,
            "  - profile={} slug={:?}",
            bool_at(evidence, "profile_found"),
            str_at(evidence, "profile_slug")
        );
    }
    if !str_at(evidence, "reason").is_empty() {
        let _ = writeln!(out, "  - {}", str_at(evidence, "reason"));
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn bool_at(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn int_at(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}
