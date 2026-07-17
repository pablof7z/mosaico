//! Canonical logical agent inventory shared by every local consumer.
//!
//! This module answers which agents exist. It deliberately does not choose a
//! harness bundle for native profiles or generic installed harnesses; hosted
//! launch intent owns that later realization step.

use crate::agent_catalog::{AgentCatalog, NativeAgentProfile};
use crate::harness::HarnessesConfig;
use crate::session::Harness;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSource {
    Configured {
        bundle: String,
        profile: Option<String>,
        per_session_key: bool,
        native_profile: Option<NativeAgentProfile>,
    },
    NativeProfile {
        profile: NativeAgentProfile,
        persist_binding: bool,
    },
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableAgent {
    /// Public selector. Cross-harness native conflicts carry a harness suffix.
    pub(crate) slug: String,
    /// Canonical agent/profile name stored in sessions and explicit bindings.
    pub(crate) agent_slug: String,
    pub(crate) harness: Harness,
    pub(crate) use_criteria: String,
    pub(crate) available_since: u64,
    pub(crate) source: AgentSource,
}

#[derive(Debug, Default)]
pub(crate) struct AgentInventory {
    pub(crate) agents: Vec<AvailableAgent>,
    pub(crate) failures: Vec<String>,
}

impl AgentInventory {
    pub(crate) fn build(
        mosaico_home: &Path,
        installed_harnesses: &[Harness],
        harnesses: &HarnessesConfig,
        catalog: &AgentCatalog,
        workspace: Option<&Path>,
    ) -> Self {
        let mut inventory = Self::default();
        let mut configured = BTreeSet::new();
        let created = crate::identity::list_invitable_agents(mosaico_home)
            .into_iter()
            .map(|(slug, _, created_at)| (slug, created_at))
            .collect::<BTreeMap<_, _>>();
        let capabilities = catalog.capabilities(workspace);

        for agent in crate::identity::list_local_agent_details(mosaico_home) {
            let bundle = agent.harness;
            let harness = match crate::harness::bundle_harness_with(harnesses, &bundle) {
                Ok(harness) => harness,
                Err(error) => {
                    inventory
                        .failures
                        .push(format!("{}: {error:#}", agent.slug));
                    continue;
                }
            };
            let native_profile = capabilities
                .iter()
                .flat_map(|capability| capability.profiles.iter())
                .find(|profile| profile.slug == agent.slug && profile.harness == harness)
                .cloned();
            let use_criteria = agent
                .byline
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    native_profile
                        .as_ref()
                        .map(|profile| profile.use_criteria.clone())
                })
                .unwrap_or_default();
            configured.insert(agent.slug.clone());
            inventory.agents.push(AvailableAgent {
                slug: agent.slug.clone(),
                agent_slug: agent.slug.clone(),
                harness,
                use_criteria,
                available_since: created.get(&agent.slug).copied().unwrap_or(0),
                source: AgentSource::Configured {
                    bundle,
                    profile: agent.profile,
                    per_session_key: agent.per_session_key,
                    native_profile,
                },
            });
        }

        for capability in capabilities {
            if configured.contains(&capability.slug) {
                continue;
            }
            let profiles = capability
                .profiles
                .into_iter()
                .filter(|profile| installed_harnesses.contains(&profile.harness))
                .collect::<Vec<_>>();
            let conflicted = profiles.len() > 1;
            for profile in profiles {
                let slug = if conflicted {
                    format!("{}-{}", capability.slug, profile.harness.agent_slug())
                } else {
                    capability.slug.clone()
                };
                inventory.agents.push(AvailableAgent {
                    slug,
                    agent_slug: capability.slug.clone(),
                    harness: profile.harness,
                    use_criteria: profile.use_criteria.clone(),
                    available_since: capability.available_since,
                    source: AgentSource::NativeProfile {
                        profile,
                        persist_binding: conflicted,
                    },
                });
            }
        }
        inventory.add_generic_agents(installed_harnesses);
        inventory.agents.sort_by(|a, b| a.slug.cmp(&b.slug));
        inventory.failures.sort();
        inventory.failures.dedup();
        inventory
    }

    pub(crate) fn find(&self, slug: &str) -> Option<&AvailableAgent> {
        self.agents.iter().find(|agent| agent.slug == slug)
    }

    pub(crate) fn profile_choices(&self, slug: &str) -> Vec<&AvailableAgent> {
        self.agents
            .iter()
            .filter(|agent| {
                agent.agent_slug == slug
                    && matches!(
                        agent.source,
                        AgentSource::NativeProfile {
                            persist_binding: true,
                            ..
                        }
                    )
            })
            .collect()
    }

    fn add_generic_agents(&mut self, installed: &[Harness]) {
        for harness in installed {
            let slug = harness.agent_slug();
            if self.agents.iter().any(|agent| agent.slug == slug) {
                continue;
            }
            self.agents.push(AvailableAgent {
                slug: slug.to_string(),
                agent_slug: slug.to_string(),
                harness: *harness,
                use_criteria: String::new(),
                available_since: 0,
                source: AgentSource::Generic,
            });
        }
    }
}

#[cfg(test)]
#[path = "agent_inventory/tests.rs"]
mod tests;
