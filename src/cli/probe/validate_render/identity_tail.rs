//! Fabric identity evidence renderer for `probe validate`.

use serde_json::Value;
use std::fmt::Write as _;

pub(super) fn render(out: &mut String, evidence: &Value) {
    let _ = writeln!(out);
    let _ = writeln!(out, "identity evidence");
    if !bool_at(evidence, "found") {
        let _ = writeln!(out, "  - {}: not found", str_at(evidence, "requested"));
    } else {
        let _ = writeln!(
            out,
            "  - {}={} -> {} profile={} identity={} derived={}",
            str_at(evidence, "kind"),
            str_at(evidence, "requested"),
            str_at(evidence, "resolved_pubkey"),
            bool_at(evidence, "profile_found"),
            bool_at(evidence, "identity_found"),
            int_at(evidence, "derived_identity_count")
        );
        if bool_at(evidence, "profile_found") {
            let _ = writeln!(
                out,
                "  - profile name={:?} slug={:?} host={:?} backend={} updated_at={}",
                str_at(evidence, "profile_name"),
                str_at(evidence, "profile_slug"),
                str_at(evidence, "profile_host"),
                bool_at(evidence, "profile_is_backend"),
                int_at(evidence, "profile_updated_at")
            );
        }
        if bool_at(evidence, "identity_found") {
            let identity = evidence.get("identity").unwrap_or(&Value::Null);
            let _ = writeln!(
                out,
                "  - identity codename={} alive={} session={} channel={} native={}",
                str_at(identity, "codename"),
                bool_at(identity, "alive"),
                str_at(identity, "session_id"),
                str_at(identity, "channel_h"),
                str_at(identity, "native_id")
            );
        }
        if bool_at(evidence, "bound_session_found") {
            let _ = writeln!(
                out,
                "  - bound session={} alive={} channel={}",
                str_at(evidence, "bound_session_id"),
                bool_at(evidence, "bound_session_alive"),
                str_at(evidence, "bound_session_channel")
            );
        }
        let _ = writeln!(
            out,
            "  - memberships={} admin_channels={}",
            int_at(evidence, "member_channel_count"),
            int_at(evidence, "admin_channel_count")
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
