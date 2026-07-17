use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct AgentUsageParams {
    since: u64,
}

pub(super) fn rpc_agent_usage(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: AgentUsageParams =
        serde_json::from_value(params.clone()).context("agent_usage params")?;
    let rows = state.with_store(|store| store.agent_usage_since(params.since))?;
    Ok(serde_json::json!({
        "agents": rows.into_iter().map(|row| serde_json::json!({
            "agent_slug": row.agent_slug,
            "recent_uses": row.recent_uses,
            "last_used": row.last_used,
        })).collect::<Vec<_>>()
    }))
}
