//! Live row samples for table-level validation discovery.

use crate::state::Store;
use anyhow::Result;
use serde_json::{json, Value};

const SAMPLE_TABLES: &[&str] = &[
    "channel_readiness_attempts",
    "channel_resolution_intents",
    "identities",
    "inbox",
    "llm_calls",
    "message_recipients",
    "messages",
    "outbox",
    "receipts",
    "relay_channel_member_sets",
    "relay_channel_members",
    "relay_channels",
    "relay_event_quarantine",
    "relay_events",
    "relay_profiles",
    "relay_status",
    "session_aliases",
    "session_channels",
    "sessions",
    "trellis_commits",
    "trellis_replay_capsules",
    "workspace_roots",
];

pub(super) fn sample_targets(store: &Store, table: &str, limit: usize) -> Result<Vec<Value>> {
    let Some(rows) = store.application_table_sample_rows(table, columns_for_table(table), limit)?
    else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .filter_map(|row| sample_target(table, row))
        .collect())
}

pub(super) fn lookup_targets(store: &Store, needle: &str, limit: usize) -> Result<Vec<Value>> {
    let mut matches = Vec::new();
    for table in SAMPLE_TABLES {
        let Some(rows) =
            store.application_table_lookup_rows(table, columns_for_table(table), needle, 3)?
        else {
            continue;
        };
        for row in rows {
            if let Some(sample) = sample_target(table, &row) {
                matches.push(sample);
                if matches.len() >= limit {
                    return Ok(matches);
                }
            }
        }
    }
    Ok(matches)
}

fn columns_for_table(table: &str) -> &'static [&'static str] {
    match table {
        "channel_readiness_attempts" => &["id", "channel_h", "outcome"],
        "channel_resolution_intents" => &["parent", "name", "channel_h"],
        "identities" => &["pubkey", "agent_slug", "session_id"],
        "inbox" => &["event_id", "target_session", "state"],
        "llm_calls" => &["id", "session_id", "provider", "model"],
        "message_recipients" => &["message_id", "recipient_pubkey", "target_session"],
        "messages" => &[
            "message_id",
            "channel_h",
            "author_pubkey",
            "native_event_id",
        ],
        "outbox" => &["local_id", "state", "event_json"],
        "receipts" => &["id", "surface", "transaction_id"],
        "relay_channel_member_sets" => &["channel_h", "role", "updated_at"],
        "relay_channel_members" => &["channel_h", "pubkey", "role"],
        "relay_channels" => &["channel_h", "name"],
        "relay_event_quarantine" => &["id", "reason"],
        "relay_events" => &["id", "kind", "channel_h"],
        "relay_profiles" => &["pubkey", "slug"],
        "relay_status" => &["session_id", "pubkey", "channel_h"],
        "session_aliases" => &["harness", "external_id_kind", "external_id"],
        "session_channels" => &["session_id", "channel_h"],
        "sessions" => &["session_id", "agent_slug", "channel_h"],
        "trellis_commits" => &["id", "surface", "transaction_id"],
        "trellis_replay_capsules" => &["id", "surface"],
        "workspace_roots" => &["channel_h", "abs_path"],
        _ => &[],
    }
}

fn sample_target(table: &str, row: &Value) -> Option<Value> {
    let target = match table {
        "channel_readiness_attempts" => format!("readiness_attempt:{}", int(row, "id")?),
        "channel_resolution_intents" => format!("channel:{}", text(row, "channel_h")?),
        "identities" => format!("identity:{}", text(row, "pubkey")?),
        "inbox" => format!("inbox:{}", text(row, "event_id")?),
        "llm_calls" => format!("llm:{}", int(row, "id")?),
        "message_recipients" => recipient_target(row)?,
        "messages" => format!("message:{}", text(row, "message_id")?),
        "outbox" => format!("outbox:{}", int(row, "local_id")?),
        "receipts" => format!("receipt:{}", int(row, "id")?),
        "relay_channel_member_sets" => {
            format!("membership_snapshot:{}", text(row, "channel_h")?)
        }
        "relay_channel_members" => membership_target(row)?,
        "relay_channels" => format!("channel:{}", text(row, "channel_h")?),
        "relay_event_quarantine" => format!("quarantine:{}", text(row, "id")?),
        "relay_events" => format!("event:{}", text(row, "id")?),
        "relay_profiles" => format!("profile:{}", text(row, "pubkey")?),
        "relay_status" => format!("status:{}", text(row, "session_id")?),
        "session_aliases" => alias_target(row)?,
        "session_channels" => {
            format!(
                "joined:{}:{}",
                text(row, "session_id")?,
                text(row, "channel_h")?
            )
        }
        "sessions" => format!("session:{}", text(row, "session_id")?),
        "trellis_commits" => format!("commit:{}", int(row, "id")?),
        "trellis_replay_capsules" => format!("capsule:{}", int(row, "id")?),
        "workspace_roots" => format!("workspace:{}", text(row, "channel_h")?),
        _ => return None,
    };
    Some(json!({
        "table": table,
        "target": target,
        "also": alternate_target(table, row),
        "row": row,
    }))
}

fn recipient_target(row: &Value) -> Option<String> {
    let base = format!(
        "recipient:{}:{}",
        text(row, "message_id")?,
        text(row, "recipient_pubkey")?
    );
    Some(match text(row, "target_session") {
        Some(session) => format!("{base}:{session}"),
        None => base,
    })
}

fn membership_target(row: &Value) -> Option<String> {
    let prefix = match text(row, "role") {
        Some("admin") => "admin",
        _ => "member",
    };
    Some(format!(
        "{prefix}:{}:{}",
        text(row, "channel_h")?,
        text(row, "pubkey")?
    ))
}

fn alias_target(row: &Value) -> Option<String> {
    Some(format!(
        "alias:{}:{}:{}",
        text(row, "harness")?,
        text(row, "external_id_kind")?,
        text(row, "external_id")?
    ))
}

fn alternate_target(table: &str, row: &Value) -> Option<String> {
    match table {
        "inbox" => text(row, "event_id").map(|id| format!("event:{id}")),
        "message_recipients" => text(row, "message_id").map(|id| format!("message:{id}")),
        "messages" => text(row, "native_event_id").map(|id| format!("event:{id}")),
        "outbox" => text(row, "event_json")
            .and_then(event_json_id)
            .map(|id| format!("event:{id}")),
        "receipts" => Some(format!(
            "txn:{}:{}",
            text(row, "surface")?,
            int(row, "transaction_id")?
        )),
        "trellis_commits" => Some(format!(
            "txn:{}:{}",
            text(row, "surface")?,
            int(row, "transaction_id")?
        )),
        _ => None,
    }
}

fn text<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn int(row: &Value, key: &str) -> Option<i64> {
    row.get(key).and_then(Value::as_i64)
}

fn event_json_id(raw: &str) -> Option<String> {
    serde_json::from_str::<Value>(raw)
        .ok()?
        .get("id")?
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
}
