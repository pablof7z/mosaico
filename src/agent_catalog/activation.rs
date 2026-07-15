use super::NativeAgentProfile;
use crate::session::Harness;
use anyhow::{Context, Result};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum NativeAgentActivation {
    NativeSelector { name: String },
    CodexRoot(CodexRootConfig),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexRootConfig {
    pub developer_instructions: String,
    pub config: Map<String, Value>,
}

pub(super) fn load(profile: &NativeAgentProfile) -> Result<NativeAgentActivation> {
    match profile.harness {
        Harness::Codex => load_codex(profile),
        Harness::ClaudeCode | Harness::Opencode => Ok(NativeAgentActivation::NativeSelector {
            name: profile.slug.clone(),
        }),
        Harness::Grok | Harness::Unknown => {
            anyhow::bail!(
                "{} has no native agent activation",
                profile.harness.as_str()
            )
        }
    }
}

fn load_codex(profile: &NativeAgentProfile) -> Result<NativeAgentActivation> {
    let body = std::fs::read_to_string(&profile.path)
        .with_context(|| format!("reading Codex agent {}", profile.path.display()))?;
    let mut table: toml::Table = toml::from_str(&body)
        .with_context(|| format!("parsing Codex agent {}", profile.path.display()))?;
    let instructions = table
        .remove("developer_instructions")
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| !value.trim().is_empty())
        .with_context(|| {
            format!(
                "Codex agent {} requires developer_instructions",
                profile.path.display()
            )
        })?;

    // These fields describe the custom-agent catalog entry. They are not
    // ordinary root-thread configuration overrides.
    table.remove("name");
    table.remove("description");
    table.remove("nickname_candidates");

    let config = serde_json::to_value(table)
        .context("converting Codex custom-agent config to app-server config")?
        .as_object()
        .cloned()
        .context("Codex custom-agent config did not serialize as an object")?;
    Ok(NativeAgentActivation::CodexRoot(CodexRootConfig {
        developer_instructions: instructions,
        config,
    }))
}
