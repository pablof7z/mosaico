use anyhow::Result;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SessionRow {
    pub(super) session_id: String,
    pub(super) handle: String,
    pub(super) agent: String,
    pub(super) workspace: String,
    pub(super) workspace_id: String,
    pub(super) channels: Vec<String>,
    pub(super) channel_ids: Vec<String>,
    pub(super) title: String,
    pub(super) activity: String,
    pub(super) busy: bool,
    pub(super) last_seen: u64,
    pub(super) host: String,
    pub(super) harness: String,
    pub(super) transport: String,
    pub(super) pty_live: bool,
    pub(super) cwd: Option<String>,
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
            format!("{title} — {activity}")
        }
    }

    pub(super) fn choice_label(&self, now: u64, max_chars: usize) -> String {
        let state = if self.busy { "working" } else { "idle" };
        let seen = if self.last_seen == 0 {
            "unknown".to_string()
        } else {
            crate::util::relative_time(self.last_seen, now)
        };
        let scope = match (self.workspace.is_empty(), self.channels.as_slice()) {
            (true, _) => "(no workspace)".to_string(),
            (false, []) => self.workspace.clone(),
            (false, [channel]) if channel == &self.workspace => self.workspace.clone(),
            (false, _) => format!("{}/{}", self.workspace, self.channels.join(",")),
        };
        compact(
            &format!(
                "@{} · {} · {} · {} · {}",
                self.handle,
                state,
                scope,
                seen,
                self.title_with_activity()
            ),
            max_chars,
        )
    }

    pub(super) fn fuzzy_score(&self, input: &str) -> Option<i64> {
        if input.is_empty() {
            return Some(0);
        }
        let channels = self.channels.join(" ");
        let channel_ids = self.channel_ids.join(" ");
        let endpoint = if self.pty_live { "pty" } else { "headless" };
        [
            (self.handle.as_str(), 4_000),
            (self.agent.as_str(), 3_000),
            (self.title.as_str(), 1_000),
            (self.activity.as_str(), 1_000),
            (self.workspace.as_str(), 2_000),
            (self.workspace_id.as_str(), 1_500),
            (channels.as_str(), 1_500),
            (channel_ids.as_str(), 1_000),
            (self.host.as_str(), 500),
            (self.harness.as_str(), 500),
            (self.transport.as_str(), 500),
            (self.cwd.as_deref().unwrap_or_default(), 500),
            (self.session_id.as_str(), 250),
            (endpoint, 250),
        ]
        .into_iter()
        .filter_map(|(field, priority)| score_field(input, field, priority))
        .max()
    }
}

fn score_field(input: &str, field: &str, priority: i64) -> Option<i64> {
    let matcher = SkimMatcherV2::default().ignore_case();
    let score = matcher.fuzzy_match(field, input)?;
    let exact_bonus = i64::from(field.to_lowercase().contains(&input.to_lowercase())) * 10_000;
    Some(score + exact_bonus + priority)
}

pub(super) async fn fetch_sessions() -> Result<Vec<SessionRow>> {
    let value = crate::cli::daemon_call_async("operator_sessions", serde_json::json!({})).await?;
    Ok(rows_from_value(&value))
}

fn rows_from_value(value: &serde_json::Value) -> Vec<SessionRow> {
    let mut rows = value["sessions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(parse_row)
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.busy
            .cmp(&a.busy)
            .then_with(|| b.pty_live.cmp(&a.pty_live))
            .then_with(|| b.last_seen.cmp(&a.last_seen))
            .then_with(|| a.handle.cmp(&b.handle))
    });
    rows
}

fn parse_row(value: &serde_json::Value) -> Option<SessionRow> {
    let session_id = value["session_id"].as_str()?.to_string();
    let endpoint = value.get("endpoint").filter(|value| !value.is_null());
    let channels = value["channels"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    Some(SessionRow {
        session_id,
        handle: value["handle"].as_str().unwrap_or("?").to_string(),
        agent: value["agent"].as_str().unwrap_or("?").to_string(),
        workspace: value["workspace"]["name"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        workspace_id: value["workspace"]["id"].as_str().unwrap_or("").to_string(),
        channels: channels
            .iter()
            .filter_map(|channel| channel["name"].as_str().map(str::to_string))
            .collect(),
        channel_ids: channels
            .iter()
            .filter_map(|channel| channel["id"].as_str().map(str::to_string))
            .collect(),
        title: value["title"].as_str().unwrap_or("").to_string(),
        activity: value["activity"].as_str().unwrap_or("").to_string(),
        busy: value["busy"].as_bool().unwrap_or(false),
        last_seen: value["last_seen"].as_u64().unwrap_or(0),
        host: value["host"].as_str().unwrap_or("").to_string(),
        harness: value["harness"].as_str().unwrap_or("").to_string(),
        transport: value["transport"].as_str().unwrap_or("").to_string(),
        pty_live: endpoint
            .and_then(|endpoint| endpoint["live"].as_bool())
            .unwrap_or(false),
        cwd: endpoint
            .and_then(|endpoint| endpoint["cwd"].as_str())
            .or_else(|| value["workspace"]["path"].as_str())
            .map(str::to_string),
    })
}

fn compact(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let prefix = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_non_pty_session() {
        let value = serde_json::json!({
            "sessions": [{
                "session_id": "s1",
                "handle": "opal-codex",
                "agent": "codex",
                "workspace": {"id": "root", "name": "tenex-edge", "path": "/repo"},
                "channels": [{"id": "root", "name": "tenex-edge"}],
                "title": "shipping the picker",
                "activity": "running tests",
                "busy": true,
                "last_seen": 12,
                "host": "laptop",
                "harness": "codex",
                "transport": "harness",
                "endpoint": null
            }]
        });

        let rows = rows_from_value(&value);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "s1");
        assert!(rows[0]
            .choice_label(20, 100)
            .contains("@opal-codex · working"));
        assert!(rows[0]
            .choice_label(20, 100)
            .contains("shipping the picker"));
        assert!(!rows[0]
            .choice_label(20, 100)
            .contains("tenex-edge/tenex-edge"));
        assert!(rows[0].fuzzy_score("repo").is_some());
        assert!(rows[0].fuzzy_score("headless").is_some());
    }

    #[test]
    fn compact_normalizes_and_truncates_long_activity() {
        assert_eq!(compact("one\n two", 20), "one two");
        assert_eq!(compact("abcdefgh", 5), "abcde…");
    }
}
