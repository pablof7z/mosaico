use crate::session_state::SessionState;
use anyhow::Result;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::cli) struct WorkspaceGroup {
    pub(in crate::cli) id: String,
    pub(in crate::cli) name: String,
    pub(in crate::cli) path: Option<String>,
    pub(in crate::cli) channels: Vec<ChannelRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::cli) struct ChannelRef {
    pub(in crate::cli) id: String,
    pub(in crate::cli) name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::cli) struct SessionRow {
    pub(in crate::cli) pubkey: String,
    pub(in crate::cli) npub: String,
    pub(in crate::cli) handle: String,
    pub(in crate::cli) agent: String,
    pub(in crate::cli) workspaces: Vec<WorkspaceGroup>,
    pub(in crate::cli) title: String,
    pub(in crate::cli) activity: String,
    pub(in crate::cli) state: SessionState,
    pub(in crate::cli) state_since: u64,
    pub(in crate::cli) running: bool,
    pub(in crate::cli) resumable: bool,
    pub(in crate::cli) created_at: u64,
    pub(in crate::cli) last_seen: u64,
    pub(in crate::cli) busy_seconds: u64,
    pub(in crate::cli) turn_started_at: u64,
    pub(in crate::cli) host: String,
    pub(in crate::cli) harness: String,
    pub(in crate::cli) pty_id: Option<String>,
    pub(in crate::cli) endpoint_live: bool,
    pub(in crate::cli) endpoint_attachable: bool,
    pub(in crate::cli) cwd: Option<String>,
    pub(in crate::cli) transport: String,
    pub(in crate::cli) takeover_available: bool,
    pub(in crate::cli) turn_open: bool,
    pub(in crate::cli) turn_count: u64,
    pub(in crate::cli) native_outcome: Option<NativeOutcomeRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::cli) struct NativeOutcomeRow {
    pub(in crate::cli) outcome: String,
    pub(in crate::cli) error_message: String,
}

impl NativeOutcomeRow {
    pub(in crate::cli) fn is_failure(&self) -> bool {
        !matches!(self.outcome.as_str(), "completed" | "started")
    }
}

impl SessionRow {
    pub(in crate::cli) fn attachable(&self) -> bool {
        self.pty_id.is_some() && self.endpoint_live && self.endpoint_attachable
    }

    pub(in crate::cli) fn can_take_over(&self) -> bool {
        self.takeover_available && !self.attachable()
    }

    pub(in crate::cli) fn stable_id(&self) -> String {
        if self.pubkey.is_empty() {
            format!("pty:{}", self.pty_id.as_deref().unwrap_or(&self.handle))
        } else {
            self.pubkey.clone()
        }
    }

    pub(in crate::cli) fn belongs_to(&self, workspace_id: &str) -> bool {
        self.workspaces
            .iter()
            .any(|workspace| workspace.id == workspace_id)
    }

    pub(in crate::cli) fn matches_workspace(&self, query: &str) -> bool {
        self.workspaces.iter().any(|workspace| {
            workspace.id.eq_ignore_ascii_case(query)
                || workspace.name.eq_ignore_ascii_case(query)
                || workspace
                    .path
                    .as_deref()
                    .is_some_and(|path| path.eq_ignore_ascii_case(query))
        })
    }

    pub(in crate::cli) fn last_activity(&self) -> u64 {
        self.created_at.max(self.last_seen).max(self.state_since)
    }

    pub(in crate::cli) fn approximate_busy_seconds(&self, now: u64) -> u64 {
        let open = if self.state == SessionState::Working && self.turn_started_at > 0 {
            now.saturating_sub(self.turn_started_at)
        } else {
            0
        };
        self.busy_seconds.saturating_add(open)
    }

    pub(in crate::cli) fn fuzzy_score(&self, input: &str) -> Option<i64> {
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

pub(in crate::cli) async fn fetch_sessions() -> Result<Vec<SessionRow>> {
    let value = crate::cli::daemon_call_async("operator_sessions", serde_json::json!({})).await?;
    Ok(rows_from_value(&value))
}

pub(in crate::cli) fn rows_from_value(value: &serde_json::Value) -> Vec<SessionRow> {
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
            .then_with(|| b.endpoint_live.cmp(&a.endpoint_live))
            .then_with(|| a.handle.to_lowercase().cmp(&b.handle.to_lowercase()))
            .then_with(|| a.pubkey.cmp(&b.pubkey))
    });
    rows
}

fn parse_row(value: &serde_json::Value) -> Option<SessionRow> {
    let pubkey = value["pubkey"].as_str()?.to_string();
    let npub = value["npub"].as_str()?.to_string();
    let endpoint = value.get("endpoint").filter(|value| !value.is_null());
    let takeover = value.get("takeover").filter(|value| !value.is_null());
    let (takeover_available, turn_open, takeover_turn_count) = match takeover {
        Some(takeover) => (
            true,
            takeover["turn_open"].as_bool()?,
            takeover["turn_count"].as_u64()?,
        ),
        None => (false, false, 0),
    };
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
    let state = value["state"]
        .as_str()
        .and_then(SessionState::parse)
        .unwrap_or_default();
    Some(SessionRow {
        pubkey,
        npub,
        handle: value["handle"].as_str().unwrap_or("?").to_string(),
        agent: value["agent"].as_str().unwrap_or("?").to_string(),
        workspaces,
        title: value["title"].as_str().unwrap_or("").to_string(),
        activity: value["activity"].as_str().unwrap_or("").to_string(),
        state,
        state_since: value["state_since"].as_u64().unwrap_or(0),
        running: value["running"]
            .as_bool()
            .unwrap_or(state != SessionState::Offline),
        resumable: value["resumable"].as_bool().unwrap_or(false),
        created_at: value["created_at"].as_u64().unwrap_or(0),
        last_seen: value["last_seen"].as_u64().unwrap_or(0),
        busy_seconds: value["busy_seconds"].as_u64().unwrap_or(0),
        turn_started_at: value["turn_started_at"].as_u64().unwrap_or(0),
        host: value["host"].as_str().unwrap_or("").to_string(),
        harness: value["harness"].as_str().unwrap_or("").to_string(),
        pty_id: endpoint
            .filter(|endpoint| endpoint["attachable"].as_bool().unwrap_or(false))
            .and_then(|endpoint| endpoint["id"].as_str())
            .map(str::to_string),
        endpoint_live: endpoint
            .and_then(|endpoint| endpoint["live"].as_bool())
            .unwrap_or(false),
        endpoint_attachable: endpoint
            .and_then(|endpoint| endpoint["attachable"].as_bool())
            .unwrap_or(false),
        cwd,
        transport: value["transport"].as_str().unwrap_or("").to_string(),
        takeover_available,
        turn_open,
        turn_count: value["turn_count"].as_u64().unwrap_or(takeover_turn_count),
        native_outcome: parse_native_outcome(value.get("native_outcome")),
    })
}

fn parse_native_outcome(value: Option<&serde_json::Value>) -> Option<NativeOutcomeRow> {
    let value = value.filter(|value| !value.is_null())?;
    Some(NativeOutcomeRow {
        outcome: value["outcome"].as_str()?.to_string(),
        error_message: value["error_message"].as_str().unwrap_or("").to_string(),
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
#[path = "data_tests.rs"]
mod tests;
