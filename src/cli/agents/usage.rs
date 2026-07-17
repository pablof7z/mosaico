use super::data::{AgentKind, AgentRow};
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

const RECENT_USAGE_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(super) struct AgentUsage {
    agent_slug: String,
    recent_uses: u64,
    last_used: u64,
}

type AgentUsageMap = HashMap<String, AgentUsage>;

pub(super) async fn fetch(now: u64) -> Result<AgentUsageMap> {
    let value = crate::cli::daemon_call_async(
        "agent_usage",
        serde_json::json!({ "since": now.saturating_sub(RECENT_USAGE_SECS) }),
    )
    .await?;
    let rows = serde_json::from_value::<Vec<AgentUsage>>(
        value.get("agents").cloned().unwrap_or_default(),
    )?;
    Ok(rows
        .into_iter()
        .map(|row| (row.agent_slug.clone(), row))
        .collect())
}

pub(super) fn ordered_rows(mut rows: Vec<AgentRow>, usage: &AgentUsageMap) -> Vec<AgentRow> {
    rows.sort_by(|left, right| {
        usage_for(usage, &right.agent_slug)
            .recent_uses
            .cmp(&usage_for(usage, &left.agent_slug).recent_uses)
            .then_with(|| {
                usage_for(usage, &right.agent_slug)
                    .last_used
                    .cmp(&usage_for(usage, &left.agent_slug).last_used)
            })
            .then_with(|| source_rank(left.kind).cmp(&source_rank(right.kind)))
            .then_with(|| left.slug.to_lowercase().cmp(&right.slug.to_lowercase()))
    });
    rows
}

fn usage_for<'a>(usage: &'a AgentUsageMap, slug: &str) -> &'a AgentUsage {
    static EMPTY: AgentUsage = AgentUsage {
        agent_slug: String::new(),
        recent_uses: 0,
        last_used: 0,
    };
    usage.get(slug).unwrap_or(&EMPTY)
}

fn source_rank(kind: AgentKind) -> u8 {
    match kind {
        AgentKind::Generic => 0,
        AgentKind::Configured => 1,
        AgentKind::NativeProfile => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Harness;

    fn row(slug: &str, agent_slug: &str, kind: AgentKind) -> AgentRow {
        AgentRow {
            slug: slug.into(),
            agent_slug: agent_slug.into(),
            description: String::new(),
            harness: Harness::Codex,
            bundle: None,
            transport: None,
            profile: None,
            per_session_key: None,
            kind,
            native_profile: None,
        }
    }

    #[test]
    fn recent_count_then_last_use_determine_order() {
        let usage = [("codex", 2, 90), ("writer", 3, 80), ("grok", 3, 95)]
            .into_iter()
            .map(|(agent_slug, recent_uses, last_used)| {
                (
                    agent_slug.to_string(),
                    AgentUsage {
                        agent_slug: agent_slug.to_string(),
                        recent_uses,
                        last_used,
                    },
                )
            })
            .collect();
        let ordered = ordered_rows(
            vec![
                row("codex", "codex", AgentKind::Generic),
                row("writer-codex", "writer", AgentKind::NativeProfile),
                row("grok", "grok", AgentKind::Generic),
            ],
            &usage,
        );

        assert_eq!(
            ordered
                .iter()
                .map(|row| row.slug.as_str())
                .collect::<Vec<_>>(),
            ["grok", "writer-codex", "codex"]
        );
    }
}
