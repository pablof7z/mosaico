use crate::agent_catalog::NativeAgentProfile;
use crate::agent_inventory::AgentSource;
use crate::harness::Transport;
use crate::session::Harness;
use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentKind {
    Configured,
    NativeProfile,
    Generic,
}

#[derive(Clone, Debug)]
pub(super) struct AgentRow {
    pub(super) slug: String,
    pub(super) agent_slug: String,
    pub(super) description: String,
    pub(super) harness: Harness,
    pub(super) bundle: Option<String>,
    pub(super) transport: Option<Transport>,
    pub(super) profile: Option<String>,
    pub(super) per_session_key: Option<bool>,
    pub(super) kind: AgentKind,
    pub(super) native_profile: Option<NativeAgentProfile>,
}

pub(super) fn load() -> Result<Vec<AgentRow>> {
    let cwd = std::env::current_dir()?;
    let inventory: crate::agent_inventory::AgentInventory = serde_json::from_value(
        crate::daemon::blocking::call("agent_inventory", serde_json::json!({ "cwd": cwd }))?,
    )?;
    Ok(inventory.agents.into_iter().map(project).collect())
}

fn project(agent: crate::agent_inventory::Agent) -> AgentRow {
    let fallback = match &agent.source {
        AgentSource::Durable { .. } => "Configured agent".to_string(),
        AgentSource::DetectedProfile { .. } => "Native agent profile".to_string(),
        AgentSource::DetectedHarness => format!("Generic {} agent", harness_name(agent.harness)),
    };
    let description = if agent.use_criteria.trim().is_empty() {
        fallback
    } else {
        agent.use_criteria
    };
    let (kind, bundle, transport, profile, per_session_key, native_profile) = match agent.source {
        AgentSource::Durable {
            pubkey: _,
            bundle,
            transport,
            profile,
            per_session_key,
            native_profile,
        } => (
            AgentKind::Configured,
            Some(bundle.clone()),
            transport,
            profile,
            Some(per_session_key),
            native_profile,
        ),
        AgentSource::DetectedProfile { profile, .. } => (
            AgentKind::NativeProfile,
            None,
            None,
            None,
            None,
            Some(profile),
        ),
        AgentSource::DetectedHarness => (AgentKind::Generic, None, None, None, None, None),
    };
    AgentRow {
        slug: agent.slug,
        agent_slug: agent.agent_slug,
        description,
        harness: agent.harness,
        bundle,
        transport,
        profile,
        per_session_key,
        kind,
        native_profile,
    }
}

pub(super) fn harness_name(harness: Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "Claude",
        Harness::Codex => "Codex",
        Harness::Opencode => "OpenCode",
        Harness::Grok => "Grok",
        Harness::Unknown => "Unknown",
    }
}

#[cfg(test)]
#[path = "data/tests.rs"]
mod tests;
