//! Workspace evidence renderer for `probe validate`.

use serde_json::Value;
use std::fmt::Write as _;

pub(super) fn render(out: &mut String, evidence: &Value) {
    let _ = writeln!(out);
    let _ = writeln!(out, "workspace evidence");
    let _ = writeln!(
        out,
        "  - channel={} root_channel={} channel_found={} direct={} inherited={}",
        str_at(evidence, "channel_h"),
        str_at(evidence, "root_channel"),
        bool_at(evidence, "channel_found"),
        bool_at(evidence, "direct_binding_found"),
        bool_at(evidence, "inherited_binding")
    );
    if bool_at(evidence, "found") {
        let _ = writeln!(
            out,
            "  - binding_channel={} path={} absolute={} exists={} dir={} updated_at={}",
            str_at(evidence, "binding_channel_h"),
            str_at(evidence, "abs_path"),
            bool_at(evidence, "path_absolute"),
            bool_at(evidence, "path_exists"),
            bool_at(evidence, "path_is_dir"),
            int_at(evidence, "updated_at")
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
