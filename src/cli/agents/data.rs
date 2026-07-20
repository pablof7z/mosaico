use crate::agent_catalog::NativeAgentProfile;
use crate::agent_inventory::AgentSource;
use crate::harness::Transport;
use crate::session::Harness;
use anyhow::Result;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::cli) enum AgentKind {
    Configured,
    NativeProfile,
    Generic,
}

#[derive(Clone, Debug)]
pub(in crate::cli) struct AgentRow {
    pub(in crate::cli) slug: String,
    pub(in crate::cli) agent_slug: String,
    pub(in crate::cli) description: String,
    pub(in crate::cli) harness: Harness,
    pub(in crate::cli) bundle: Option<String>,
    pub(in crate::cli) transport: Option<Transport>,
    pub(in crate::cli) profile: Option<String>,
    pub(in crate::cli) per_session_key: Option<bool>,
    pub(in crate::cli) kind: AgentKind,
    pub(in crate::cli) native_profile: Option<NativeAgentProfile>,
}

impl AgentRow {
    pub(in crate::cli) fn fuzzy_score(&self, input: &str) -> Option<i64> {
        if input.is_empty() {
            return Some(0);
        }
        let matcher = SkimMatcherV2::default().ignore_case();
        [
            (self.slug.as_str(), 4_000),
            (self.description.as_str(), 2_000),
            (harness_name(self.harness), 750),
        ]
        .into_iter()
        .filter_map(|(field, priority)| {
            let score = matcher.fuzzy_match(field, input)?;
            let exact = i64::from(field.to_lowercase().contains(&input.to_lowercase())) * 10_000;
            Some(score + exact + priority)
        })
        .max()
    }

    pub(in crate::cli) fn has_configured(&self) -> bool {
        self.kind == AgentKind::Configured
    }

    pub(in crate::cli) fn has_native_profile(&self) -> bool {
        self.native_profile.is_some()
    }

    pub(in crate::cli) fn summary(&self, max_chars: usize) -> String {
        crate::agent_about::compact(&self.description, max_chars)
    }
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

pub(in crate::cli) fn harness_name(harness: Harness) -> &'static str {
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
