//! Inbox target validation for inbound event delivery/orchestration rows.

use super::report::{bool_at, int_at, str_at};
use super::DaemonState;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(super) struct InboxTarget {
    event_prefix: String,
    target_session: Option<String>,
}

pub(super) fn inbox_target(target: &str) -> Option<InboxTarget> {
    if let Some(rest) = target.strip_prefix("inbox:") {
        let (event_prefix, target_session) = split_colon_target(rest)?;
        return Some(InboxTarget {
            event_prefix: event_prefix.to_string(),
            target_session: target_session.map(str::to_string),
        });
    }
    let rest = target.strip_prefix("inbox/")?;
    let (event_prefix, target_session) = match rest.split_once('/') {
        Some((event, target)) => (event, Some(target)),
        None => (rest, None),
    };
    (!event_prefix.trim().is_empty()).then(|| InboxTarget {
        event_prefix: event_prefix.to_string(),
        target_session: target_session
            .filter(|target| !target.trim().is_empty())
            .map(str::to_string),
    })
}

pub(super) fn inbox_evidence(
    state: &Arc<DaemonState>,
    target: &str,
    parsed: &InboxTarget,
) -> Value {
    let result = state.with_store(|store| {
        let rows = match parsed.target_session.as_deref() {
            Some(target_session) => {
                store.inbox_by_event_prefix_and_target(&parsed.event_prefix, target_session)?
            }
            None => store.inbox_by_event_prefix(&parsed.event_prefix)?,
        };
        let session_rows = rows
            .iter()
            .map(|row| {
                if synthetic_target(&row.target_session) {
                    Ok(None)
                } else {
                    store.get_session(&row.target_session)
                }
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok::<_, anyhow::Error>((rows, session_rows))
    });
    let (rows, session_rows) = match result {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "target": target,
                "event_prefix": parsed.event_prefix,
                "target_session": parsed.target_session,
                "kind": "inbox",
                "supported": true,
                "found": false,
                "error": e.to_string(),
                "summary": "inbox evidence could not read durable ledger",
                "reason": e.to_string(),
            });
        }
    };

    let distinct_events = rows
        .iter()
        .map(|row| row.event_id.as_str())
        .collect::<BTreeSet<_>>();
    let row_values = rows
        .iter()
        .zip(session_rows.iter())
        .take(10)
        .map(|(row, session)| row_json(row, session.as_ref()))
        .collect::<Vec<_>>();
    let found = !rows.is_empty();
    let ambiguous = distinct_events.len() > 1;
    let failed_count = rows.iter().filter(|row| failed_state(&row.state)).count();
    let pending_count = rows.iter().filter(|row| row.state == "pending").count();
    let processing_count = rows.iter().filter(|row| row.state == "processing").count();
    let delivered_count = rows
        .iter()
        .filter(|row| delivered_state(&row.state))
        .count();
    let ok =
        found && !ambiguous && failed_count == 0 && pending_count == 0 && processing_count == 0;

    json!({
        "target": target,
        "event_prefix": parsed.event_prefix,
        "target_session": parsed.target_session,
        "kind": "inbox",
        "supported": true,
        "found": found,
        "ambiguous": ambiguous,
        "event_count": distinct_events.len(),
        "row_count": rows.len(),
        "pending_count": pending_count,
        "processing_count": processing_count,
        "delivered_count": delivered_count,
        "failed_count": failed_count,
        "rows": row_values,
        "ok": ok,
        "summary": summary(&parsed.event_prefix, parsed.target_session.as_deref(), rows.len(), ambiguous, pending_count, processing_count, failed_count),
        "reason": reason(found, ambiguous, pending_count, processing_count, failed_count),
    })
}

pub(super) fn push_inbox_check(
    checks: &mut Vec<Value>,
    limitations: &mut Vec<String>,
    evidence: &Value,
) {
    let status = if !str_at(evidence, "error").is_empty() || int_at(evidence, "failed_count") > 0 {
        "failed"
    } else if bool_at(evidence, "ok") {
        "passed"
    } else {
        "not_proven"
    };
    checks.push(json!({
        "name": "inbox",
        "status": status,
        "summary": str_at(evidence, "summary"),
    }));
    if status != "passed" && !str_at(evidence, "reason").is_empty() {
        limitations.push(str_at(evidence, "reason").to_string());
    }
}

fn split_colon_target(rest: &str) -> Option<(&str, Option<&str>)> {
    if rest.trim().is_empty() {
        return None;
    }
    match rest.split_once(':') {
        Some((event, target)) if !event.trim().is_empty() && !target.trim().is_empty() => {
            Some((event, Some(target)))
        }
        Some((event, _)) if !event.trim().is_empty() => Some((event, None)),
        Some(_) => None,
        None => Some((rest, None)),
    }
}

fn row_json(row: &crate::state::InboxRow, session: Option<&crate::state::Session>) -> Value {
    json!({
        "event_id": row.event_id,
        "target_session": row.target_session,
        "target_kind": target_kind(&row.target_session),
        "state": row.state,
        "from_pubkey": row.from_pubkey,
        "channel_h": row.channel_h,
        "body_len": row.body.chars().count(),
        "body_preview": body_preview(&row.body),
        "created_at": row.created_at,
        "delivered_at": row.delivered_at,
        "session_row_found": session.is_some(),
        "session_alive": session.is_some_and(|s| s.alive),
        "agent_slug": session.map(|s| s.agent_slug.as_str()).unwrap_or(""),
    })
}

fn target_kind(target_session: &str) -> &'static str {
    if target_session == "management" {
        "management"
    } else if target_session.starts_with("orchestration:") {
        "orchestration"
    } else if target_session.starts_with("offline-mention:") {
        "offline_mention"
    } else {
        "session"
    }
}

fn synthetic_target(target_session: &str) -> bool {
    !matches!(target_kind(target_session), "session")
}

fn delivered_state(state: &str) -> bool {
    matches!(
        state,
        "delivered" | "injected" | "echo_consumed" | "offline_handled"
    )
}

fn failed_state(state: &str) -> bool {
    matches!(state, "failed" | "error" | "rejected")
}

fn body_preview(body: &str) -> String {
    const LIMIT: usize = 96;
    let trimmed = body.trim();
    let mut preview = trimmed.chars().take(LIMIT).collect::<String>();
    if trimmed.chars().count() > LIMIT {
        preview.push_str("...");
    }
    preview
}

fn summary(
    event_prefix: &str,
    target_session: Option<&str>,
    row_count: usize,
    ambiguous: bool,
    pending_count: usize,
    processing_count: usize,
    failed_count: usize,
) -> String {
    let suffix = target_session
        .map(|target| format!(" for `{target}`"))
        .unwrap_or_default();
    if row_count == 0 {
        return format!("inbox `{event_prefix}` has no durable inbound row{suffix}");
    }
    if ambiguous {
        return format!("inbox `{event_prefix}` matches multiple inbound events");
    }
    if failed_count > 0 {
        return format!("inbox `{event_prefix}` has {failed_count} failed row(s){suffix}");
    }
    if pending_count > 0 || processing_count > 0 {
        return format!("inbox `{event_prefix}` has unfinished inbound row(s){suffix}");
    }
    format!("inbox `{event_prefix}` has {row_count} completed inbound row(s){suffix}")
}

fn reason(
    found: bool,
    ambiguous: bool,
    pending_count: usize,
    processing_count: usize,
    failed_count: usize,
) -> &'static str {
    if !found {
        "no inbound ledger row matched this event id prefix"
    } else if ambiguous {
        "event id prefix matches multiple inbound events; use a longer id"
    } else if failed_count > 0 {
        "one or more inbound rows records a failed/rejected state"
    } else if pending_count > 0 {
        "inbound event is queued but has not been delivered to the target yet"
    } else if processing_count > 0 {
        "orchestration/management target is still processing or its lease has not been retried"
    } else {
        ""
    }
}
