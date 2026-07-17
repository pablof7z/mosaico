use crate::agent_catalog::{AgentCatalog, NativeAgentProfile};
use crate::agent_inventory::{AgentInventory, AgentSource};
use crate::harness::{HarnessesConfig, Transport};
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
    let home = crate::config::mosaico_home();
    let harnesses = HarnessesConfig::load()?;
    let installed = crate::config::detect_available_harnesses()?;
    let catalog = AgentCatalog::discover(
        &crate::agent_catalog::DiscoveryRoots::installed()?,
        std::slice::from_ref(&cwd),
    )?;
    let inventory = AgentInventory::build(&home, &installed, &harnesses, &catalog, Some(&cwd));
    Ok(inventory
        .agents
        .into_iter()
        .map(|agent| project(agent, &harnesses))
        .collect())
}

fn project(agent: crate::agent_inventory::AvailableAgent, harnesses: &HarnessesConfig) -> AgentRow {
    let fallback = match &agent.source {
        AgentSource::Configured { .. } => "Configured agent".to_string(),
        AgentSource::NativeProfile { .. } => "Native agent profile".to_string(),
        AgentSource::Generic => format!("Generic {} agent", harness_name(agent.harness)),
    };
    let description = if agent.use_criteria.trim().is_empty() {
        fallback
    } else {
        agent.use_criteria
    };
    let (kind, bundle, transport, profile, per_session_key, native_profile) = match agent.source {
        AgentSource::Configured {
            bundle,
            profile,
            per_session_key,
            native_profile,
        } => (
            AgentKind::Configured,
            Some(bundle.clone()),
            harnesses.get(&bundle).map(|bundle| bundle.transport),
            profile,
            Some(per_session_key),
            native_profile,
        ),
        AgentSource::NativeProfile { profile, .. } => (
            AgentKind::NativeProfile,
            None,
            None,
            None,
            None,
            Some(profile),
        ),
        AgentSource::Generic => (AgentKind::Generic, None, None, None, None, None),
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
