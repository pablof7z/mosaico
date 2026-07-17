use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

const RECENT_USAGE_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(super) struct AgentUsage {
    pub(super) agent_slug: String,
    pub(super) recent_uses: u64,
    pub(super) last_used: u64,
}

pub(super) type AgentUsageMap = HashMap<String, AgentUsage>;

static EMPTY_AGENT_USAGE: AgentUsage = AgentUsage {
    agent_slug: String::new(),
    recent_uses: 0,
    last_used: 0,
};

pub(super) async fn fetch_agent_usage(now: u64) -> Result<AgentUsageMap> {
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

pub(super) fn ordered_agents<'a>(
    inventory: &'a crate::agent_inventory::AgentInventory,
    usage: &AgentUsageMap,
) -> Vec<&'a crate::agent_inventory::AvailableAgent> {
    let mut agents = inventory.agents.iter().collect::<Vec<_>>();
    agents.sort_by(|a, b| {
        usage_for(usage, b)
            .recent_uses
            .cmp(&usage_for(usage, a).recent_uses)
            .then_with(|| {
                usage_for(usage, b)
                    .last_used
                    .cmp(&usage_for(usage, a).last_used)
            })
            .then_with(|| source_rank(a.source).cmp(&source_rank(b.source)))
            .then_with(|| a.slug.to_lowercase().cmp(&b.slug.to_lowercase()))
    });
    agents
}

pub(super) fn usage_for<'a>(
    usage: &'a AgentUsageMap,
    agent: &crate::agent_inventory::AvailableAgent,
) -> &'a AgentUsage {
    usage.get(&agent.agent_slug).unwrap_or(&EMPTY_AGENT_USAGE)
}

fn source_rank(source: crate::agent_inventory::AgentSource) -> u8 {
    match source {
        crate::agent_inventory::AgentSource::Harness => 0,
        crate::agent_inventory::AgentSource::Configured => 1,
        crate::agent_inventory::AgentSource::NativeProfile => 2,
    }
}
