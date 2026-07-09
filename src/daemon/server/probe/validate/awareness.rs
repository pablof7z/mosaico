//! Awareness/`who` validation evidence.

use super::report::{bool_at, str_at};
use super::DaemonState;
use crate::who_snapshot::WhoSource;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

pub(super) fn awareness_evidence(
    state: &Arc<DaemonState>,
    params: &Value,
    target: &str,
    requested: &str,
) -> Value {
    let scope = match requested_scope(params, requested) {
        Scope::Missing(reason) => {
            return json!({
                "target": target,
                "supported": true,
                "found": false,
                "channel_confirmed": false,
                "summary": "awareness target has no resolved channel/channel context",
                "reason": reason,
            });
        }
        scope => scope,
    };
    let now = crate::util::now_secs();
    let host = state.host.clone();
    match state.with_store(|store| build_evidence(store, target, scope, now, &host)) {
        Ok(v) => v,
        Err(e) => json!({
            "target": target,
            "supported": true,
            "found": false,
            "channel_confirmed": false,
            "error": e.to_string(),
            "summary": "awareness snapshot could not be loaded",
            "reason": e.to_string(),
        }),
    }
}

pub(super) fn push_awareness_check(
    checks: &mut Vec<Value>,
    limitations: &mut Vec<String>,
    evidence: &Value,
) {
    let failed = !str_at(evidence, "error").is_empty();
    let passed = bool_at(evidence, "found") && bool_at(evidence, "channel_confirmed");
    let status = if failed {
        "failed"
    } else if passed || bool_at(evidence, "all_roots") {
        "passed"
    } else {
        "not_proven"
    };
    checks.push(json!({
        "name": "awareness",
        "status": status,
        "summary": str_at(evidence, "summary"),
    }));
    if status != "passed" && !str_at(evidence, "reason").is_empty() {
        limitations.push(str_at(evidence, "reason").to_string());
    }
}

fn build_evidence(
    store: &crate::state::Store,
    target: &str,
    scope: Scope,
    now: u64,
    host: &str,
) -> anyhow::Result<Value> {
    let current_root = scope.channel();
    let snapshot = crate::who_snapshot::load_who_snapshot(store, current_root, now, host)?;
    let rows = snapshot.rows.len();
    let local_rows = snapshot
        .rows
        .iter()
        .filter(|row| row.source == WhoSource::Local)
        .count();
    let peer_rows = rows.saturating_sub(local_rows);
    let fresh_rows = snapshot.rows.iter().filter(|row| row.fresh).count();
    let spawnable = snapshot.spawnable.len();
    let known_channels = store.list_channels()?.len();

    if scope.all_roots() {
        return Ok(json!({
            "target": target,
            "supported": true,
            "found": true,
            "all_roots": true,
            "channel_confirmed": true,
            "known_channel_count": known_channels,
            "row_count": rows,
            "local_row_count": local_rows,
            "peer_row_count": peer_rows,
            "fresh_row_count": fresh_rows,
            "spawnable_count": spawnable,
            "other_root_count": snapshot.other_roots.len(),
            "summary": format!(
                "all-channel awareness has {rows} live row(s) across {known_channels} known channel(s); {spawnable} local spawnable agent(s) are separate"
            ),
        }));
    }

    let channel_h = current_root.unwrap_or_default();
    let channel = store.get_channel(channel_h)?;
    let confirmed = channel.is_some();
    let membership_snapshot = store.has_channel_membership_snapshot(channel_h)?;
    let root_channel = store.root_channel_of(channel_h)?;
    let members = store.list_channel_members(channel_h)?;
    let admin_count = members.iter().filter(|m| m.role == "admin").count();
    let summary = if confirmed {
        format!(
            "awareness for channel `{channel_h}` has {rows} live row(s); {spawnable} local spawnable agent(s) are separate"
        )
    } else {
        format!("awareness target channel `{channel_h}` is not confirmed in relay channel cache")
    };
    Ok(json!({
        "target": target,
        "supported": true,
        "found": confirmed,
        "all_roots": false,
        "channel_h": channel_h,
        "channel_confirmed": confirmed,
        "channel_name": channel.as_ref().map(|c| c.human_name().unwrap_or(&c.name)),
        "parent": channel.as_ref().map(|c| c.parent.as_str()).unwrap_or(""),
        "root_channel": root_channel.unwrap_or_default(),
        "membership_snapshot": membership_snapshot,
        "member_count": members.len(),
        "admin_count": admin_count,
        "row_count": rows,
        "local_row_count": local_rows,
        "peer_row_count": peer_rows,
        "fresh_row_count": fresh_rows,
        "spawnable_count": spawnable,
        "other_root_count": snapshot.other_roots.len(),
        "summary": summary,
        "reason": (!confirmed).then_some(
            "awareness must be backed by confirmed relay channel metadata; local/default names and spawnable agents are not channel presence"
        ),
    }))
}

enum Scope {
    Channel(String),
    AllRoots,
    Missing(String),
}

impl Scope {
    fn channel(&self) -> Option<&str> {
        match self {
            Scope::Channel(channel) => Some(channel.as_str()),
            Scope::AllRoots | Scope::Missing(_) => None,
        }
    }

    fn all_roots(&self) -> bool {
        matches!(self, Scope::AllRoots)
    }
}

fn requested_scope(params: &Value, requested: &str) -> Scope {
    let requested = requested.trim();
    if requested == "*" {
        return Scope::AllRoots;
    }
    if !requested.is_empty() {
        return Scope::Channel(requested.to_string());
    }
    if params
        .get("all_roots")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Scope::AllRoots;
    }
    for key in ["group", "channel"] {
        if let Some(value) = params
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            return Scope::Channel(value.to_string());
        }
    }
    if let Some(cwd) = params
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        if let Ok(channel) = crate::workspace::resolve(Path::new(cwd)) {
            return Scope::Channel(channel);
        }
    }
    Scope::Missing(
        "validate awareness needs a target channel, caller channel, channel, cwd, or `awareness:*`"
            .to_string(),
    )
}
