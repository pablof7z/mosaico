//! Canonical logical agent inventory shared by every local consumer.
//!
//! This module answers which agents exist. It deliberately does not choose a
//! transport for native profiles or generic installed harnesses; hosted
//! launch intent owns that later realization step.

use crate::agent_catalog::{AgentCatalog, NativeAgentProfile};
use crate::session::Harness;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum AgentSource {
    Durable {
        pubkey: Option<String>,
        profile: Option<String>,
        preset: Option<String>,
        per_session_key: bool,
        native_profile: Option<NativeAgentProfile>,
    },
    DetectedProfile {
        profile: NativeAgentProfile,
        persist_binding: bool,
    },
    DetectedHarness,
}

/// The canonical daemon-owned representation of a launchable agent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Agent {
    /// Public selector. Cross-harness native conflicts carry a harness suffix.
    pub(crate) slug: String,
    /// Canonical agent/profile name stored in sessions and explicit bindings.
    pub(crate) agent_slug: String,
    pub(crate) harness: Harness,
    pub(crate) use_criteria: String,
    pub(crate) available_since: u64,
    pub(crate) source: AgentSource,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct AgentInventory {
    pub(crate) agents: Vec<Agent>,
    pub(crate) failures: Vec<String>,
}

impl AgentInventory {
    pub(crate) fn build(
        mosaico_home: &Path,
        installed_harnesses: &[Harness],
        catalog: &AgentCatalog,
        workspace: Option<&Path>,
    ) -> Self {
        let mut inventory = Self::default();
        let mut configured = BTreeMap::new();
        let entries = crate::identity::keystore_entries(mosaico_home);
        let created = entries
            .iter()
            .map(|entry| (entry.slug.clone(), entry.created_at))
            .collect::<BTreeMap<_, _>>();
        let capabilities = catalog.capabilities(workspace);

        for agent in entries {
            let harness = Harness::from_str(&agent.harness);
            // A configured slug may be a sanitized form of the native
            // profile's free-text name (see `crate::slug::slugify`), so fall
            // back to a sanitized comparison when the exact one misses.
            let native_profile = capabilities
                .iter()
                .flat_map(|capability| capability.profiles.iter())
                .find(|profile| {
                    profile.harness == harness
                        && (profile.slug == agent.slug
                            || crate::slug::slugify(&profile.slug) == agent.slug)
                })
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
            configured.insert(agent.slug.clone(), harness);
            inventory.agents.push(Agent {
                slug: agent.slug.clone(),
                agent_slug: agent.slug.clone(),
                harness,
                use_criteria,
                available_since: created.get(&agent.slug).copied().unwrap_or(0),
                source: AgentSource::Durable {
                    pubkey: agent.pubkey,
                    profile: agent.profile,
                    preset: agent.preset,
                    per_session_key: agent.per_session_key,
                    native_profile,
                },
            });
        }

        for capability in capabilities {
            let configured_harness = configured
                .get(&capability.slug)
                .or_else(|| configured.get(&crate::slug::slugify(&capability.slug)))
                .copied();
            let profiles = capability
                .profiles
                .into_iter()
                .filter(|profile| installed_harnesses.contains(&profile.harness))
                .filter(|profile| configured_harness != Some(profile.harness))
                .collect::<Vec<_>>();
            let conflicted = configured_harness.is_some() || profiles.len() > 1;
            for profile in profiles {
                let slug = if conflicted {
                    format!("{}-{}", capability.slug, profile.harness.agent_slug())
                } else {
                    capability.slug.clone()
                };
                inventory.agents.push(Agent {
                    slug,
                    agent_slug: capability.slug.clone(),
                    harness: profile.harness,
                    use_criteria: profile.use_criteria.clone(),
                    available_since: capability.available_since,
                    source: AgentSource::DetectedProfile {
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

    pub(crate) fn find(&self, slug: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.slug == slug)
    }

    pub(crate) fn durable_agent_for_pubkey(&self, pubkey: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| {
            matches!(
                &agent.source,
                AgentSource::Durable {
                    pubkey: Some(configured),
                    ..
                } if configured == pubkey
            )
        })
    }

    pub(crate) fn profile_choices(&self, slug: &str) -> Vec<&Agent> {
        self.agents
            .iter()
            .filter(|agent| {
                agent.agent_slug == slug
                    && matches!(
                        agent.source,
                        AgentSource::DetectedProfile {
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
            self.agents.push(Agent {
                slug: slug.to_string(),
                agent_slug: slug.to_string(),
                harness: *harness,
                use_criteria: String::new(),
                available_since: 0,
                source: AgentSource::DetectedHarness,
            });
        }
    }
}

#[cfg(test)]
#[path = "agent_inventory/tests.rs"]
mod tests;
