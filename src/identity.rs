//! Agent keystore (M1 §4).
//!
//! `--agent <slug>` resolves an agent configuration and persisted Nostr keypair.
//! Normal sessions derive their own signer from the backend root; agents with
//! `perSessionKey:false` sign with this persisted key across sequential runs.
//!
//! NOTE: agent keypairs live under `<mosaico_home>/agents/<slug>.json`, which
//! defaults to `~/.mosaico/agents/`. `mosaico_home()` defaults to `~/.mosaico`.

use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod keys;
mod local_agent;
pub use keys::{derive_session_keys, new_session_signer_salt, SessionIdentity};
pub use local_agent::{
    add_local_agent, agent_launch_config, list_advertised_agents, list_invitable_agents,
    list_local_agent_details, list_local_agents, list_local_pubkeys, remove_local_agent,
    set_local_agent_byline, AgentLaunchConfig, LocalAgent,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredKey {
    slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_key: Option<String>, // hex; durable agents only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_key: Option<String>, // hex; durable agents only
    created_at: u64,
    /// One-line "when to use this agent" note, surfaced in `my session`'s
    /// capability inventory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    byline: Option<String>,
    /// When false, this persisted key signs every fresh session for the agent.
    /// The default keeps the normal per-session derived-key contract.
    #[serde(default = "default_per_session_key", rename = "perSessionKey")]
    per_session_key: bool,
    /// Required bundle name in `~/.mosaico/harnesses.json`.
    harness: String,
    /// Optional harness-specific profile name. Code translates it according to
    /// the configured `(harness, transport)` driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
}

const fn default_per_session_key() -> bool {
    true
}

impl StoredKey {
    fn effective_byline(&self) -> Option<String> {
        self.byline
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

/// A resolved agent identity plus its launch bundle/profile selection.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub slug: String,
    pub keys: Option<Keys>,
    pub per_session_key: bool,
    pub harness: String,
    pub profile: Option<String>,
}

impl AgentIdentity {
    pub fn pubkey_hex(&self) -> Option<String> {
        self.keys.as_ref().map(|keys| keys.public_key().to_hex())
    }

    pub fn per_session(slug: &str, harness: &str) -> Self {
        Self {
            slug: slug.to_string(),
            keys: None,
            per_session_key: true,
            harness: harness.to_string(),
            profile: None,
        }
    }
}

pub fn is_configured(mosaico_home: &Path, slug: &str) -> bool {
    key_path(mosaico_home, slug).is_file()
}

fn agents_dir(mosaico_home: &Path) -> PathBuf {
    mosaico_home.join("agents")
}

fn key_path(mosaico_home: &Path, slug: &str) -> PathBuf {
    agents_dir(mosaico_home).join(format!("{slug}.json"))
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        bail!("invalid agent slug {slug:?}: use [A-Za-z0-9._-]");
    }
    Ok(())
}

/// Load an agent, creating it on first observed session with an explicit bundle
/// and optional harness-specific profile. Existing configuration is never overwritten by
/// observations; operator updates go through [`add_local_agent`].
pub fn load_or_create(
    mosaico_home: &Path,
    slug: &str,
    harness: &str,
    profile: Option<&str>,
    now: u64,
) -> Result<AgentIdentity> {
    validate_slug(slug)?;
    let harness = validate_config_name("harness", harness)?;
    let profile = normalize_optional_config_name("profile", profile)?;
    let path = key_path(mosaico_home, slug);
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading key {}", path.display()))?;
        let mut stored: StoredKey =
            serde_json::from_str(&s).with_context(|| format!("parsing key {}", path.display()))?;
        let keys = stored.identity_keys()?;
        if stored.drop_redundant_session_key() {
            atomic_write(&path, &serde_json::to_string_pretty(&stored)?)?;
            tracing::info!(slug, path = %path.display(), "removed redundant per-session agent key");
        }
        return Ok(AgentIdentity {
            slug: slug.to_string(),
            keys,
            per_session_key: stored.per_session_key,
            harness: stored.harness,
            profile: stored.profile,
        });
    }

    let stored = StoredKey {
        slug: slug.to_string(),
        secret_key: None,
        public_key: None,
        created_at: now,
        byline: None,
        per_session_key: true,
        harness,
        profile,
    };
    std::fs::create_dir_all(agents_dir(mosaico_home))
        .with_context(|| format!("creating {}", agents_dir(mosaico_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    tracing::info!(slug, path = %path.display(), "keyless agent configuration created");
    Ok(AgentIdentity {
        slug: slug.to_string(),
        keys: None,
        per_session_key: stored.per_session_key,
        harness: stored.harness,
        profile: stored.profile,
    })
}

/// Load an existing configured agent. Spawn/resume paths use this so an
/// unconfigured role cannot silently acquire an inferred launch policy.
pub fn load(mosaico_home: &Path, slug: &str) -> Result<AgentIdentity> {
    validate_slug(slug)?;
    let path = key_path(mosaico_home, slug);
    let s = std::fs::read_to_string(&path)
        .with_context(|| format!("reading configured agent {}", path.display()))?;
    let mut stored: StoredKey =
        serde_json::from_str(&s).with_context(|| format!("parsing agent {}", path.display()))?;
    let keys = stored.identity_keys()?;
    if stored.drop_redundant_session_key() {
        atomic_write(&path, &serde_json::to_string_pretty(&stored)?)?;
        tracing::info!(slug, path = %path.display(), "removed redundant per-session agent key");
    }
    Ok(AgentIdentity {
        slug: stored.slug,
        keys,
        per_session_key: stored.per_session_key,
        harness: stored.harness,
        profile: stored.profile,
    })
}

impl StoredKey {
    fn identity_keys(&self) -> Result<Option<Keys>> {
        if self.per_session_key {
            return Ok(None);
        }
        let secret = self
            .secret_key
            .as_deref()
            .context("perSessionKey:false requires secret_key")?;
        let public = self
            .public_key
            .as_deref()
            .context("perSessionKey:false requires public_key")?;
        let keys = Keys::parse(secret)
            .with_context(|| format!("parsing durable secret key for {:?}", self.slug))?;
        if keys.public_key().to_hex() != public {
            anyhow::bail!(
                "durable agent {:?} public_key does not match secret_key",
                self.slug
            );
        }
        Ok(Some(keys))
    }

    fn drop_redundant_session_key(&mut self) -> bool {
        if !self.per_session_key || (self.secret_key.is_none() && self.public_key.is_none()) {
            return false;
        }
        self.secret_key = None;
        self.public_key = None;
        true
    }
}

fn validate_config_name(field: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("agent {field} must not be empty");
    }
    Ok(value.to_string())
}

fn normalize_optional_config_name(field: &str, value: Option<&str>) -> Result<Option<String>> {
    value
        .map(|value| validate_config_name(field, value))
        .transpose()
}

/// Write via a temp file + rename so a crash never leaves a half-written key.
fn atomic_write(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
