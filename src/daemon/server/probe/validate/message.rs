//! Message/read-model validation evidence for `probe validate`.

use super::report::{bool_at, str_at};
use super::DaemonState;
use serde_json::{json, Value};
use std::sync::Arc;

const MESSAGE_LIMITATION: &str = "message validation proves the local canonical channel read model; relay acceptance is proven only when the row carries accepted sync state and a native event id";

pub(super) fn message_target(target: &str) -> Option<&str> {
    target
        .strip_prefix("message:")
        .or_else(|| target.strip_prefix("message/"))
        .or_else(|| target.strip_prefix("msg:"))
        .or_else(|| target.strip_prefix("msg/"))
        .and_then(|rest| rest.split('/').next())
        .filter(|id| !id.trim().is_empty())
}

pub(super) fn message_evidence(
    state: &Arc<DaemonState>,
    target: &str,
    requested_id: &str,
) -> Value {
    match state.with_store(|store| {
        let Some(row) = store.get_message_by_prefix(requested_id)? else {
            return Ok::<Value, anyhow::Error>(json!({
                "target": target,
                "requested_id": requested_id,
                "kind": "message",
                "supported": true,
                "found": false,
                "summary": format!("message `{requested_id}` is not in the canonical channel read model"),
                "reason": "no messages row matched this local message id or native event id prefix",
            }));
        };

        let channel = store.get_channel(&row.channel_h)?;
        let recipients = store.message_recipients(&row.message_id)?;
        let delivered_count = recipients.iter().filter(|r| r.delivered_at.is_some()).count();

        Ok::<Value, anyhow::Error>(json!({
            "target": target,
            "requested_id": requested_id,
            "message_id": row.message_id,
            "native_event_id": row.native_event_id,
            "kind": "message",
            "supported": true,
            "found": true,
            "channel_h": row.channel_h,
            "channel_confirmed": channel.is_some(),
            "thread_id": row.thread_id,
            "author_pubkey": row.author_pubkey,
            "direction": row.direction,
            "sync_state": row.sync_state,
            "error": row.error,
            "created_at": row.created_at,
            "body_len": row.body.chars().count(),
            "body_preview": body_preview(&row.body),
            "recipient_count": recipients.len(),
            "delivered_recipient_count": delivered_count,
            "pending_recipient_count": recipients.len().saturating_sub(delivered_count),
            "summary": summary(&row),
            "reason": reason(&row, channel.is_some()),
        }))
    }) {
        Ok(v) => v,
        Err(e) => json!({
            "target": target,
            "requested_id": requested_id,
            "kind": "message",
            "supported": true,
            "found": false,
            "summary": format!("message `{requested_id}` evidence failed: {e}"),
            "reason": e.to_string(),
            "error": e.to_string(),
        }),
    }
}

pub(super) fn push_message_check(
    checks: &mut Vec<Value>,
    limitations: &mut Vec<String>,
    evidence: &Value,
) {
    let status = if !str_at(evidence, "error").is_empty() {
        "failed"
    } else if !bool_at(evidence, "found") {
        "not_proven"
    } else if failed_sync(evidence) {
        "failed"
    } else if provisional_sync(evidence) {
        "not_proven"
    } else {
        "passed"
    };
    checks.push(json!({
        "name": "message",
        "status": status,
        "summary": str_at(evidence, "summary"),
    }));
    if status != "passed" && !str_at(evidence, "reason").is_empty() {
        limitations.push(str_at(evidence, "reason").to_string());
    } else if bool_at(evidence, "found") && !bool_at(evidence, "channel_confirmed") {
        limitations.push(
            "message row exists, but its channel is not materialized from relay kind:39000"
                .to_string(),
        );
    } else if status == "passed" {
        limitations.push(MESSAGE_LIMITATION.to_string());
    }
}

fn summary(row: &crate::state::Message) -> String {
    if let Some(error) = row.error.as_deref().filter(|s| !s.is_empty()) {
        return format!("message `{}` failed: {error}", row.message_id);
    }
    match row.sync_state.as_str() {
        "accepted" => format!(
            "message `{}` is accepted in channel `{}`",
            row.message_id, row.channel_h
        ),
        state => format!(
            "message `{}` has sync_state `{state}` in channel `{}`",
            row.message_id, row.channel_h
        ),
    }
}

fn reason(row: &crate::state::Message, channel_confirmed: bool) -> &'static str {
    if row.error.as_deref().is_some_and(|s| !s.is_empty()) || is_failed_state(&row.sync_state) {
        return "message row records a failed/rejected sync state";
    }
    if is_provisional_state(&row.sync_state) || row.native_event_id.is_none() {
        return "message row exists locally but relay/native event materialization is not proven";
    }
    if !channel_confirmed {
        return "message row references a channel that is not materialized from relay kind:39000";
    }
    MESSAGE_LIMITATION
}

fn failed_sync(evidence: &Value) -> bool {
    !str_at(evidence, "error").is_empty() || is_failed_state(str_at(evidence, "sync_state"))
}

fn provisional_sync(evidence: &Value) -> bool {
    is_provisional_state(str_at(evidence, "sync_state"))
        || str_at(evidence, "native_event_id").is_empty()
}

fn is_failed_state(state: &str) -> bool {
    matches!(state, "failed" | "error" | "rejected")
}

fn is_provisional_state(state: &str) -> bool {
    matches!(state, "pending" | "queued" | "sending" | "draft")
}

fn body_preview(body: &str) -> String {
    const LIMIT: usize = 96;
    let mut preview = body.trim().chars().take(LIMIT).collect::<String>();
    if body.trim().chars().count() > LIMIT {
        preview.push_str("...");
    }
    preview
}
