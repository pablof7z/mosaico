use super::Page;
use anyhow::Result;
use serde_json::{json, Value};
use std::fmt::Write as _;

pub(super) fn text(page: &Page, now: u64) -> String {
    let mut out = String::new();
    for row in &page.sessions {
        let workspaces = workspace_names(row);
        let busy = busy_hint(row.approximate_busy_seconds(now));
        let activity = activity_hint(row.last_activity(), now);
        let turns = format!(
            "{} turn{}",
            row.turn_count,
            if row.turn_count == 1 { "" } else { "s" }
        );
        let _ = writeln!(
            out,
            "@{}  {} · busy {} · {} · {} · {}",
            row.handle, row.state, busy, turns, workspaces, activity
        );
        let work = work_summary(row);
        if !work.is_empty() {
            let _ = writeln!(out, "  {work}");
        }
        let _ = writeln!(out, "  open: mosaico {}", row.handle);
    }
    if page.sessions.is_empty() {
        out.push_str("No matching sessions.\n");
    }
    let shown_end = page.offset.saturating_add(page.sessions.len());
    let _ = writeln!(
        out,
        "Showing {}-{} of {}{}",
        if page.sessions.is_empty() {
            0
        } else {
            page.offset + 1
        },
        shown_end,
        page.total,
        page.next_offset()
            .map(|offset| format!("; continue with --offset {offset}"))
            .unwrap_or_default()
    );
    out
}

pub(super) fn json(page: &Page, now: u64) -> Result<String> {
    let sessions = page
        .sessions
        .iter()
        .map(|row| {
            json!({
                "pubkey": row.pubkey,
                "npub": row.npub,
                "handle": row.handle,
                "agent": row.agent,
                "title": row.title,
                "activity": row.activity,
                "state": row.state,
                "created_at": row.created_at,
                "last_activity_at": row.last_activity(),
                "running": row.running,
                "resumable": row.resumable,
                "turn_count": row.turn_count,
                "busy_seconds": row.approximate_busy_seconds(now),
                "busy": busy_hint(row.approximate_busy_seconds(now)),
                "host": row.host,
                "harness": row.harness,
                "transport": row.transport,
                "cwd": row.cwd,
                "workspaces": row.workspaces.iter().map(|workspace| json!({
                    "id": workspace.id,
                    "name": workspace.name,
                    "path": workspace.path,
                    "channels": workspace.channels.iter().map(|channel| json!({
                        "id": channel.id,
                        "name": channel.name,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<Value>>(),
                "open_command": format!("mosaico {}", row.handle),
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&json!({
        "sessions": sessions,
        "page": {
            "total": page.total,
            "limit": page.limit,
            "offset": page.offset,
            "has_more": page.has_more(),
            "next_offset": page.next_offset(),
        },
        "scope": {
            "workspace": page.workspace,
        },
    }))?)
}

fn workspace_names(row: &super::SessionRow) -> String {
    if row.workspaces.is_empty() {
        return "(no workspace)".to_string();
    }
    row.workspaces
        .iter()
        .map(|workspace| {
            if workspace.name.is_empty() {
                workspace.id.as_str()
            } else {
                workspace.name.as_str()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn work_summary(row: &super::SessionRow) -> String {
    let title = row.title.trim();
    let activity = row.activity.trim();
    match (title.is_empty(), activity.is_empty() || activity == title) {
        (true, _) => activity.to_string(),
        (false, true) => title.to_string(),
        (false, false) => format!("{title} — {activity}"),
    }
}

fn activity_hint(last_activity: u64, now: u64) -> String {
    if last_activity == 0 {
        "activity unknown".to_string()
    } else {
        crate::util::relative_time(last_activity, now)
    }
}

fn busy_hint(seconds: u64) -> String {
    let value = if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    };
    format!("~{value}")
}
