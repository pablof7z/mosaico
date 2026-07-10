//! Channel, awareness, and message evidence renderers.

use serde_json::Value;
use std::fmt::Write as _;

pub(super) fn render_channel(out: &mut String, channel: &Value) {
    let _ = writeln!(out);
    let _ = writeln!(out, "channel evidence");
    let status = if bool_at(channel, "found") {
        "materialized"
    } else {
        "not materialized"
    };
    let _ = writeln!(out, "  - {}: {status}", str_at(channel, "channel_h"));
    if bool_at(channel, "found") {
        let _ = writeln!(
            out,
            "  - name={:?} parent={:?} root={:?}",
            str_at(channel, "human_name"),
            str_at(channel, "parent"),
            str_at(channel, "root_channel")
        );
        let _ = writeln!(
            out,
            "  - members={} admins={} membership_snapshot={}",
            int_at(channel, "member_count"),
            int_at(channel, "admin_count"),
            bool_at(channel, "membership_snapshot")
        );
    }
    render_readiness(out, channel);
    if str_at(channel, "kind") != "readiness" && !str_at(channel, "reason").is_empty() {
        let _ = writeln!(out, "  - {}", str_at(channel, "reason"));
    }
}

fn render_readiness(out: &mut String, channel: &Value) {
    let count = int_at(channel, "session_start_count");
    let failures = int_at(channel, "channel_ready_failure_count");
    let provider_attempts = int_at(channel, "provider_attempt_count");
    if str_at(channel, "kind") != "readiness" && count == 0 && provider_attempts == 0 {
        return;
    }
    let _ = writeln!(
        out,
        "  - readiness ok={} attempts={} channel_ready={} failures={} provider_attempts={} provider_degraded={}",
        bool_at(channel, "readiness_ok"),
        count,
        int_at(channel, "session_start_channel_ready_count"),
        failures,
        provider_attempts,
        int_at(channel, "provider_degraded_count")
    );
    if !str_at(channel, "readiness_summary").is_empty() {
        let _ = writeln!(out, "  - {}", str_at(channel, "readiness_summary"));
    }
    if let Some(rows) = channel.get("session_start_rows").and_then(Value::as_array) {
        for row in rows.iter().take(4) {
            let _ = writeln!(
                out,
                "  - session_start/{} action={} channel_ready={} spawn={}",
                str_at(row, "session_id"),
                str_at(row, "action"),
                bool_at(row, "has_channel_ready_intent"),
                bool_at(row, "has_spawn_intent")
            );
            if !str_at(row, "failure_stage").is_empty() || !str_at(row, "failure_error").is_empty()
            {
                let _ = writeln!(
                    out,
                    "      failed at {}: {}",
                    str_at(row, "failure_stage"),
                    str_at(row, "failure_error")
                );
            }
        }
        if rows.len() > 4 {
            let _ = writeln!(out, "  - ... {} more session_start row(s)", rows.len() - 4);
        }
    }
    if let Some(rows) = channel
        .get("provider_attempt_rows")
        .and_then(Value::as_array)
    {
        for row in rows.iter().take(4) {
            let _ = writeln!(
                out,
                "  - provider_attempt/{} outcome={} member={} parent={:?}",
                int_at(row, "id"),
                str_at(row, "outcome"),
                str_at(row, "expect_member"),
                str_at(row, "parent_hint")
            );
            if !str_at(row, "reason").is_empty() {
                let _ = writeln!(out, "      reason: {}", str_at(row, "reason"));
            }
        }
        if rows.len() > 4 {
            let _ = writeln!(out, "  - ... {} more provider attempt(s)", rows.len() - 4);
        }
    }
    if !str_at(channel, "readiness_reason").is_empty() {
        let _ = writeln!(out, "  - {}", str_at(channel, "readiness_reason"));
    }
}

pub(super) fn render_awareness(out: &mut String, awareness: &Value) {
    let _ = writeln!(out);
    let _ = writeln!(out, "awareness evidence");
    if bool_at(awareness, "all_workspaces") || bool_at(awareness, "all_roots") {
        let _ = writeln!(
            out,
            "  - all workspaces: {} known channel(s)",
            int_at(awareness, "known_channel_count")
        );
    } else {
        let status = if bool_at(awareness, "channel_confirmed") {
            "confirmed"
        } else {
            "not confirmed"
        };
        let _ = writeln!(out, "  - {}: {status}", str_at(awareness, "channel_h"));
        if bool_at(awareness, "channel_confirmed") {
            let _ = writeln!(
                out,
                "  - name={:?} parent={:?} root={:?}",
                str_at(awareness, "channel_name"),
                str_at(awareness, "parent"),
                str_at(awareness, "root_channel")
            );
            let _ = writeln!(
                out,
                "  - members={} admins={} membership_snapshot={}",
                int_at(awareness, "member_count"),
                int_at(awareness, "admin_count"),
                bool_at(awareness, "membership_snapshot")
            );
        }
    }
    let _ = writeln!(
        out,
        "  - live_rows={} local={} peer={} fresh={} spawnable_local={}",
        int_at(awareness, "row_count"),
        int_at(awareness, "local_row_count"),
        int_at(awareness, "peer_row_count"),
        int_at(awareness, "fresh_row_count"),
        int_at(awareness, "spawnable_count")
    );
    if !str_at(awareness, "reason").is_empty() {
        let _ = writeln!(out, "  - {}", str_at(awareness, "reason"));
    }
}

pub(super) fn render_message(out: &mut String, evidence: &Value) {
    let _ = writeln!(out);
    let _ = writeln!(out, "message evidence");
    if !bool_at(evidence, "found") {
        let _ = writeln!(out, "  - {}: not found", str_at(evidence, "requested_id"));
    } else {
        let channel = if bool_at(evidence, "channel_confirmed") {
            "confirmed"
        } else {
            "not confirmed"
        };
        let _ = writeln!(
            out,
            "  - {}: {} channel={} ({channel})",
            str_at(evidence, "message_id"),
            str_at(evidence, "sync_state"),
            str_at(evidence, "channel_h")
        );
        let _ = writeln!(
            out,
            "  - direction={} author_session={:?} native_event_id={:?}",
            str_at(evidence, "direction"),
            str_at(evidence, "author_session"),
            str_at(evidence, "native_event_id")
        );
        let _ = writeln!(
            out,
            "  - recipients={} delivered={} pending={}",
            int_at(evidence, "recipient_count"),
            int_at(evidence, "delivered_recipient_count"),
            int_at(evidence, "pending_recipient_count")
        );
        let _ = writeln!(
            out,
            "  - body_len={} preview={:?}",
            int_at(evidence, "body_len"),
            str_at(evidence, "body_preview")
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
