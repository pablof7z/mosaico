use super::super::super::DaemonState;
use super::readiness;
use serde_json::{json, Value};
use std::sync::Arc;

pub(in crate::daemon::server::probe::validate) fn channel_evidence(
    state: &Arc<DaemonState>,
    target: &str,
    channel_h: &str,
) -> Value {
    let mode = if is_readiness_target(target) {
        "readiness"
    } else {
        "channel"
    };
    let readiness = readiness::evidence(state, channel_h);
    match state.with_store(|store| {
        let Some(channel) = store.get_channel(channel_h)? else {
            return Ok::<Value, anyhow::Error>(json!({
                "target": target,
                "channel_h": channel_h,
                "kind": mode,
                "supported": true,
                "found": false,
                "summary": format!("channel `{channel_h}` is not materialized from relay kind:39000"),
                "reason": readiness::channel_reason(false, false, &readiness),
                "readiness_ok": false,
                "readiness_summary": readiness::summary(false, false, &readiness),
                "readiness_reason": readiness::readiness_reason(false, false, &readiness),
                "session_start_count": readiness.rows.len(),
                "session_start_channel_ready_count": readiness.channel_ready_count,
                "session_start_failed_count": readiness.failed_count,
                "channel_ready_failure_count": readiness.channel_ready_failure_count,
                "provider_attempt_count": readiness.provider_attempt_count,
                "provider_degraded_count": readiness.provider_degraded_count,
                "provider_attempt_rows": readiness.provider_rows,
                "session_start_rows": readiness.rows,
            }));
        };

        let members = store.list_channel_members(channel_h)?;
        let admin_count = members.iter().filter(|m| m.role == "admin").count();
        let member_count = members.len();
        let membership_snapshot = store.has_channel_membership_snapshot(channel_h)?;
        let root_channel = store.root_channel_of(channel_h)?;
        let is_root = root_channel.as_deref() == Some(channel_h);
        let human_name = channel.human_name().map(str::to_string);

        Ok::<Value, anyhow::Error>(json!({
            "target": target,
            "channel_h": channel_h,
            "kind": mode,
            "supported": true,
            "found": true,
            "summary": format!("channel `{channel_h}` is materialized from relay kind:39000"),
            "reason": readiness::channel_reason(true, membership_snapshot, &readiness),
            "name": channel.name,
            "human_name": human_name,
            "about": channel.about,
            "parent": channel.parent,
            "root_channel": root_channel,
            "is_root": is_root,
            "is_archived": channel.is_archived(),
            "membership_snapshot": membership_snapshot,
            "admin_count": admin_count,
            "member_count": member_count,
            "created_at": channel.created_at,
            "updated_at": channel.updated_at,
            "readiness_ok": membership_snapshot,
            "readiness_summary": readiness::summary(true, membership_snapshot, &readiness),
            "readiness_reason": readiness::readiness_reason(true, membership_snapshot, &readiness),
            "session_start_count": readiness.rows.len(),
            "session_start_channel_ready_count": readiness.channel_ready_count,
            "session_start_failed_count": readiness.failed_count,
            "channel_ready_failure_count": readiness.channel_ready_failure_count,
            "provider_attempt_count": readiness.provider_attempt_count,
            "provider_degraded_count": readiness.provider_degraded_count,
            "provider_attempt_rows": readiness.provider_rows,
            "session_start_rows": readiness.rows,
        }))
    }) {
        Ok(v) => v,
        Err(e) => json!({
            "target": target,
            "channel_h": channel_h,
            "kind": mode,
            "supported": true,
            "found": false,
            "summary": format!("channel `{channel_h}` evidence failed: {e}"),
            "reason": readiness::channel_reason(false, false, &readiness),
            "error": e.to_string(),
            "readiness_ok": false,
            "readiness_summary": readiness::summary(false, false, &readiness),
            "readiness_reason": e.to_string(),
            "session_start_count": readiness.rows.len(),
            "session_start_channel_ready_count": readiness.channel_ready_count,
            "session_start_failed_count": readiness.failed_count,
            "channel_ready_failure_count": readiness.channel_ready_failure_count,
            "provider_attempt_count": readiness.provider_attempt_count,
            "provider_degraded_count": readiness.provider_degraded_count,
            "provider_attempt_rows": readiness.provider_rows,
            "session_start_rows": readiness.rows,
        }),
    }
}

pub(in crate::daemon::server::probe::validate) fn push_channel_check(
    checks: &mut Vec<Value>,
    limitations: &mut Vec<String>,
    evidence: &Value,
) {
    let readiness = str_at(evidence, "kind") == "readiness";
    let status = if readiness {
        if !str_at(evidence, "error").is_empty() {
            "failed"
        } else if bool_at(evidence, "readiness_ok") {
            "passed"
        } else if int_at(evidence, "channel_ready_failure_count") > 0
            || int_at(evidence, "provider_degraded_count") > 0
        {
            "failed"
        } else {
            "not_proven"
        }
    } else if bool_at(evidence, "found") {
        "passed"
    } else {
        "not_proven"
    };
    checks.push(json!({
        "name": if readiness { "channel_readiness" } else { "channel" },
        "status": status,
        "summary": if readiness {
            str_at(evidence, "readiness_summary")
        } else {
            str_at(evidence, "summary")
        },
    }));
    let reason = if readiness {
        str_at(evidence, "readiness_reason")
    } else {
        str_at(evidence, "reason")
    };
    if !reason.is_empty() {
        limitations.push(reason.to_string());
    }
}

fn is_readiness_target(target: &str) -> bool {
    target.starts_with("readiness:")
        || target.starts_with("readiness/")
        || target.starts_with("channel_ready:")
        || target.starts_with("channel_ready/")
        || target.starts_with("channel-ready:")
        || target.starts_with("channel-ready/")
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
