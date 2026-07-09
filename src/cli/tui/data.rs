use anyhow::Result;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SessionRow {
    pub(super) session_id: String,
    pub(super) agent: String,
    pub(super) channels: Vec<String>,
    pub(super) title: String,
    pub(super) activity: String,
    pub(super) busy: bool,
    pub(super) last_seen: u64,
    pub(super) updated_at: u64,
    pub(super) pty_id: Option<String>,
    pub(super) pty_live: bool,
    pub(super) cwd: Option<String>,
    pub(super) command: Vec<String>,
}

impl SessionRow {
    pub(super) fn display_title(&self) -> &str {
        let trimmed = self.title.trim();
        if trimmed.is_empty() {
            "(untitled)"
        } else {
            trimmed
        }
    }

    pub(super) fn title_with_activity(&self) -> String {
        let title = self.display_title();
        let activity = self.activity.trim();
        if activity.is_empty() || activity == title || title == "(untitled)" {
            title.to_string()
        } else {
            format!("{title} - {activity}")
        }
    }

    pub(super) async fn resolve_pty_id(&self) -> Result<Option<String>> {
        if self.pty_live {
            if let Some(id) = &self.pty_id {
                return Ok(Some(id.clone()));
            }
        }
        if self.session_id.is_empty() {
            return Ok(None);
        }
        let v = super::super::daemon_call_async(
            "pty_attach",
            serde_json::json!({ "session": self.session_id }),
        )
        .await?;
        Ok(v["pty_id"].as_str().map(str::to_string))
    }
}

pub(super) async fn fetch_sessions() -> Result<Vec<SessionRow>> {
    let sessions =
        super::super::daemon_call_async("agents_list_sessions", serde_json::json!({})).await?;
    let ptys = super::super::daemon_call_async("pty_status", serde_json::json!({})).await?;
    Ok(merge_session_values(&sessions, &ptys))
}

fn merge_session_values(sessions: &serde_json::Value, ptys: &serde_json::Value) -> Vec<SessionRow> {
    let mut rows: BTreeMap<String, SessionRow> = BTreeMap::new();
    for value in sessions["sessions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let Some(session_id) = value["session_id"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let channel = value["channel"].as_str().unwrap_or("").to_string();
        let updated_at = value["updated_at"].as_u64().unwrap_or(0);
        let last_seen = value["last_seen"].as_u64().unwrap_or(updated_at);
        let entry = rows
            .entry(session_id.to_string())
            .or_insert_with(|| SessionRow {
                session_id: session_id.to_string(),
                agent: value["agent"].as_str().unwrap_or("?").to_string(),
                title: value["title"].as_str().unwrap_or("").to_string(),
                activity: value["activity"].as_str().unwrap_or("").to_string(),
                busy: value["busy"].as_bool().unwrap_or(false),
                last_seen,
                updated_at,
                ..SessionRow::default()
            });
        if !channel.is_empty() && !entry.channels.contains(&channel) {
            entry.channels.push(channel);
        }
        if updated_at >= entry.updated_at {
            entry.agent = value["agent"].as_str().unwrap_or("?").to_string();
            entry.title = value["title"].as_str().unwrap_or("").to_string();
            entry.activity = value["activity"].as_str().unwrap_or("").to_string();
            entry.busy = value["busy"].as_bool().unwrap_or(false);
            entry.updated_at = updated_at;
        }
        entry.last_seen = entry.last_seen.max(last_seen);
    }

    for value in ptys["endpoints"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let Some(pty_id) = value["pty_id"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let session_id = value["session_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(pty_id);
        let entry = rows
            .entry(session_id.to_string())
            .or_insert_with(|| SessionRow {
                session_id: session_id.to_string(),
                agent: value["agent"].as_str().unwrap_or("?").to_string(),
                channels: value["root"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| vec![s.to_string()])
                    .unwrap_or_default(),
                title: "PTY session".to_string(),
                ..SessionRow::default()
            });
        entry.pty_id = Some(pty_id.to_string());
        entry.pty_live = value["live"].as_bool().unwrap_or(false);
        entry.cwd = value["cwd"].as_str().map(str::to_string);
        entry.command = value["command"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if entry.agent == "?" {
            entry.agent = value["agent"].as_str().unwrap_or("?").to_string();
        }
    }

    let mut out = rows.into_values().collect::<Vec<_>>();
    out.sort_by(|a, b| {
        b.pty_live
            .cmp(&a.pty_live)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
            .then_with(|| a.agent.cmp(&b.agent))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_dedups_status_rows_and_marks_pty() {
        let sessions = serde_json::json!({
            "sessions": [
                {"session_id": "s1", "agent": "codex", "channel": "root", "title": "old", "activity": "", "busy": false, "last_seen": 10, "updated_at": 10},
                {"session_id": "s1", "agent": "codex", "channel": "side", "title": "new", "activity": "testing", "busy": true, "last_seen": 12, "updated_at": 12}
            ]
        });
        let ptys = serde_json::json!({
            "endpoints": [{"pty_id": "pty-1", "session_id": "s1", "agent": "codex", "root": "root", "cwd": "/tmp", "command": ["codex"], "live": true}]
        });

        let rows = merge_session_values(&sessions, &ptys);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].channels, vec!["root", "side"]);
        assert_eq!(rows[0].title_with_activity(), "new - testing");
        assert_eq!(rows[0].pty_id.as_deref(), Some("pty-1"));
        assert!(rows[0].pty_live);
    }
}
