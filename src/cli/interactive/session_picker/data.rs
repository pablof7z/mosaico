use crate::session_state::SessionState;
use anyhow::Result;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct WorkspaceGroup {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) path: Option<String>,
    pub(super) channels: Vec<ChannelRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ChannelRef {
    pub(super) id: String,
    pub(super) name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SessionRow {
    pub(super) pubkey: String,
    pub(super) npub: String,
    pub(super) handle: String,
    pub(super) agent: String,
    pub(super) workspaces: Vec<WorkspaceGroup>,
    pub(super) title: String,
    pub(super) activity: String,
    pub(super) state: SessionState,
    pub(super) last_seen: u64,
    pub(super) host: String,
    pub(super) harness: String,
    pub(super) pty_id: Option<String>,
    pub(super) pty_live: bool,
    pub(super) cwd: Option<String>,
}

impl SessionRow {
    pub(super) fn fuzzy_score(&self, input: &str) -> Option<i64> {
        if input.is_empty() {
            return Some(0);
        }
        let workspaces = self
            .workspaces
            .iter()
            .flat_map(|workspace| {
                std::iter::once(workspace.id.as_str())
                    .chain(std::iter::once(workspace.name.as_str()))
                    .chain(workspace.path.as_deref())
                    .chain(
                        workspace
                            .channels
                            .iter()
                            .flat_map(|channel| [channel.id.as_str(), channel.name.as_str()]),
                    )
            })
            .collect::<Vec<_>>()
            .join(" ");
        [
            (self.handle.as_str(), 4_000),
            (self.agent.as_str(), 3_000),
            (self.title.as_str(), 1_000),
            (self.activity.as_str(), 1_000),
            (workspaces.as_str(), 2_000),
            (self.host.as_str(), 500),
            (self.harness.as_str(), 500),
            (self.cwd.as_deref().unwrap_or_default(), 500),
            (self.npub.as_str(), 750),
            (self.pubkey.as_str(), 250),
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
        state_rank(b.state)
            .cmp(&state_rank(a.state))
            .then_with(|| b.pty_live.cmp(&a.pty_live))
            .then_with(|| b.last_seen.cmp(&a.last_seen))
            .then_with(|| a.handle.cmp(&b.handle))
    });
    rows
}

fn parse_row(value: &serde_json::Value) -> Option<SessionRow> {
    let pubkey = value["pubkey"].as_str()?.to_string();
    let npub = value["npub"].as_str()?.to_string();
    let endpoint = value.get("endpoint").filter(|value| !value.is_null());
    let workspaces = value["workspaces"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(parse_workspace)
        .collect::<Vec<_>>();
    let cwd = endpoint
        .and_then(|endpoint| endpoint["cwd"].as_str())
        .map(str::to_string)
        .or_else(|| {
            workspaces
                .iter()
                .find_map(|workspace| workspace.path.clone())
        });
    Some(SessionRow {
        pubkey,
        npub,
        handle: value["handle"].as_str().unwrap_or("?").to_string(),
        agent: value["agent"].as_str().unwrap_or("?").to_string(),
        workspaces,
        title: value["title"].as_str().unwrap_or("").to_string(),
        activity: value["activity"].as_str().unwrap_or("").to_string(),
        state: value["state"]
            .as_str()
            .and_then(SessionState::parse)
            .unwrap_or_default(),
        last_seen: value["last_seen"].as_u64().unwrap_or(0),
        host: value["host"].as_str().unwrap_or("").to_string(),
        harness: value["harness"].as_str().unwrap_or("").to_string(),
        pty_id: endpoint
            .and_then(|endpoint| endpoint["pty_id"].as_str())
            .map(str::to_string),
        pty_live: endpoint
            .and_then(|endpoint| endpoint["live"].as_bool())
            .unwrap_or(false),
        cwd,
    })
}

fn state_rank(state: SessionState) -> u8 {
    match state {
        SessionState::Working => 3,
        SessionState::Idle => 2,
        SessionState::Suspended => 1,
        SessionState::Offline => 0,
    }
}

fn parse_workspace(value: &serde_json::Value) -> Option<WorkspaceGroup> {
    Some(WorkspaceGroup {
        id: value["id"].as_str()?.to_string(),
        name: value["name"].as_str().unwrap_or("").to_string(),
        path: value["path"].as_str().map(str::to_string),
        channels: value["channels"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|channel| {
                Some(ChannelRef {
                    id: channel["id"].as_str()?.to_string(),
                    name: channel["name"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grouped_workspace_and_attach_endpoint() {
        let value = serde_json::json!({
            "sessions": [{
                "pubkey": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "npub": "npub1publicselector",
                "handle": "opal-codex",
                "agent": "codex",
                "workspaces": [{
                    "id": "root", "name": "tenex-edge", "path": "/repo",
                    "channels": [{"id": "root", "name": "tenex-edge"}]
                }],
                "title": "shipping the picker",
                "activity": "running tests",
                "state": "working",
                "last_seen": 12,
                "host": "laptop",
                "harness": "codex",
                "endpoint": {"pty_id": "pty-1", "live": true, "cwd": "/repo"}
            }]
        });

        let rows = rows_from_value(&value);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].pubkey,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(rows[0].npub, "npub1publicselector");
        assert_eq!(rows[0].handle, "opal-codex");
        assert_eq!(rows[0].title, "shipping the picker");
        assert_eq!(rows[0].state, SessionState::Working);
        assert!(rows[0].fuzzy_score("npub1public").is_some());
        assert!(rows[0].fuzzy_score("repo").is_some());
        assert_eq!(rows[0].workspaces[0].name, "tenex-edge");
        assert_eq!(rows[0].pty_id.as_deref(), Some("pty-1"));
        assert!(rows[0].pty_live);
    }

    #[test]
    fn parses_live_unbound_endpoint_for_attach() {
        let value = serde_json::json!({
            "sessions": [{
                "pubkey": "",
                "npub": "",
                "handle": "codex",
                "agent": "codex",
                "workspaces": [{
                    "id": "root", "name": "tenex-edge", "path": "/repo",
                    "channels": [{"id": "root", "name": "tenex-edge"}]
                }],
                "title": "codex --yolo",
                "activity": "/repo",
                "state": "suspended",
                "last_seen": 0,
                "host": "laptop",
                "harness": "codex",
                "bound": false,
                "endpoint": {"pty_id": "pty-orphan", "live": true, "cwd": "/repo"}
            }]
        });

        let rows = rows_from_value(&value);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].handle, "codex");
        assert_eq!(rows[0].pty_id.as_deref(), Some("pty-orphan"));
        assert!(rows[0].pty_live);
    }
}
