//! Native harness agent discovery.
//!
//! Harness-owned files remain the source of truth for behavior. This module
//! only projects the metadata Mosaico needs to advertise and route a role.

use crate::session::Harness;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod activation;
mod parse;
pub use activation::{CodexRootConfig, NativeAgentActivation};
use parse::discover_dir;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentScope {
    Global,
    Workspace(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativeAgentProfile {
    pub slug: String,
    pub use_criteria: String,
    pub harness: Harness,
    pub path: PathBuf,
    pub scope: AgentScope,
    pub modified_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentCapability {
    pub slug: String,
    pub use_criteria: String,
    pub profiles: Vec<NativeAgentProfile>,
    pub available_since: u64,
}

#[derive(Debug, Clone)]
pub struct DiscoveryRoots {
    pub codex: PathBuf,
    pub claude: PathBuf,
    pub opencode: PathBuf,
}

impl DiscoveryRoots {
    pub fn for_user_home(home: &Path) -> Self {
        Self {
            codex: home.join(".codex/agents"),
            claude: home.join(".claude/agents"),
            opencode: home.join(".config/opencode/agents"),
        }
    }

    pub fn installed() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is required to discover installed harness agents")?;
        let mut roots = Self::for_user_home(&home);
        if let Some(codex_home) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
            roots.codex = PathBuf::from(codex_home).join("agents");
        }
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            roots.opencode = PathBuf::from(xdg).join("opencode/agents");
        }
        Ok(roots)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCatalog {
    profiles: Vec<NativeAgentProfile>,
}

impl AgentCatalog {
    pub fn discover(roots: &DiscoveryRoots, workspaces: &[PathBuf]) -> Result<Self> {
        let mut profiles = Vec::new();
        discover_dir(
            &roots.codex,
            Harness::Codex,
            AgentScope::Global,
            &mut profiles,
        )?;
        discover_dir(
            &roots.claude,
            Harness::ClaudeCode,
            AgentScope::Global,
            &mut profiles,
        )?;
        discover_dir(
            &roots.opencode,
            Harness::Opencode,
            AgentScope::Global,
            &mut profiles,
        )?;
        for workspace in workspaces {
            discover_dir(
                &workspace.join(".codex/agents"),
                Harness::Codex,
                AgentScope::Workspace(workspace.clone()),
                &mut profiles,
            )?;
            discover_dir(
                &workspace.join(".claude/agents"),
                Harness::ClaudeCode,
                AgentScope::Workspace(workspace.clone()),
                &mut profiles,
            )?;
            discover_dir(
                &workspace.join(".opencode/agents"),
                Harness::Opencode,
                AgentScope::Workspace(workspace.clone()),
                &mut profiles,
            )?;
        }
        profiles.sort_by_key(profile_key);
        validate_unique_sources(&profiles)?;
        Ok(Self { profiles })
    }

    pub fn capabilities(&self, workspace: Option<&Path>) -> Vec<AgentCapability> {
        let mut selected = BTreeMap::<(String, String), NativeAgentProfile>::new();
        for profile in &self.profiles {
            let in_scope = match (&profile.scope, workspace) {
                (AgentScope::Global, _) => true,
                (AgentScope::Workspace(root), Some(active)) => root == active,
                (AgentScope::Workspace(_), None) => false,
            };
            if !in_scope {
                continue;
            }
            let key = (profile.slug.clone(), profile.harness.as_str().to_string());
            match selected.get(&key) {
                Some(existing) if matches!(existing.scope, AgentScope::Workspace(_)) => {}
                _ => {
                    selected.insert(key, profile.clone());
                }
            }
        }

        let mut grouped = BTreeMap::<String, Vec<NativeAgentProfile>>::new();
        for profile in selected.into_values() {
            grouped
                .entry(profile.slug.clone())
                .or_default()
                .push(profile);
        }
        grouped
            .into_iter()
            .map(|(slug, profiles)| {
                let available_since = profiles.iter().map(|p| p.modified_at).min().unwrap_or(0);
                let use_criteria = profiles
                    .iter()
                    .find(|p| !p.use_criteria.is_empty())
                    .map(|p| p.use_criteria.clone())
                    .unwrap_or_default();
                AgentCapability {
                    slug,
                    use_criteria,
                    profiles,
                    available_since,
                }
            })
            .collect()
    }

    pub fn slugs(&self) -> Vec<String> {
        self.profiles
            .iter()
            .map(|profile| profile.slug.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn resolve(
        &self,
        slug: &str,
        workspace: Option<&Path>,
        preferred_harness: Option<Harness>,
    ) -> Result<NativeAgentProfile> {
        let capability = self
            .capabilities(workspace)
            .into_iter()
            .find(|candidate| candidate.slug == slug)
            .with_context(|| format!("no installed harness agent named {slug:?}"))?;
        if let Some(harness) = preferred_harness {
            return capability
                .profiles
                .into_iter()
                .find(|profile| profile.harness == harness)
                .with_context(|| {
                    format!(
                        "installed agent {slug:?} has no {} implementation",
                        harness.as_str()
                    )
                });
        }
        if capability.profiles.len() == 1 {
            return Ok(capability.profiles.into_iter().next().unwrap());
        }
        let harnesses = capability
            .profiles
            .iter()
            .map(|p| p.harness.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "installed agent {slug:?} is provided by multiple harnesses ({harnesses}); configure an explicit harness binding"
        )
    }
}

impl NativeAgentProfile {
    pub fn activation(&self) -> Result<NativeAgentActivation> {
        activation::load(self)
    }
}

/// Permanently delete one exact, already-discovered native profile source file.
///
/// Callers resolve the profile through the catalog first, which disambiguates
/// same-named profiles by harness and scope without accepting an arbitrary path.
pub fn remove_native_profile(profile: &NativeAgentProfile) -> Result<bool> {
    match std::fs::remove_file(&profile.path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("deleting native agent profile {}", profile.path.display())),
    }
}

fn profile_key(profile: &NativeAgentProfile) -> (String, String, String) {
    (
        profile.slug.clone(),
        profile.harness.as_str().to_string(),
        profile.path.to_string_lossy().into_owned(),
    )
}

fn validate_unique_sources(profiles: &[NativeAgentProfile]) -> Result<()> {
    let mut seen = BTreeMap::<(String, String, String), &Path>::new();
    for profile in profiles {
        let scope = match &profile.scope {
            AgentScope::Global => String::from("global"),
            AgentScope::Workspace(root) => root.to_string_lossy().into_owned(),
        };
        let key = (
            scope,
            profile.harness.as_str().to_string(),
            profile.slug.clone(),
        );
        if let Some(first) = seen.insert(key, &profile.path) {
            anyhow::bail!(
                "duplicate {} agent {:?} in the same scope: {} and {}",
                profile.harness.as_str(),
                profile.slug,
                first.display(),
                profile.path.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "agent_catalog/tests.rs"]
mod tests;
