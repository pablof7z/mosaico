//! Agent keystore (M1 §4).
//!
//! `--agent <slug>` resolves an agent configuration and persisted Nostr keypair.
//! Normal sessions derive their own signer from the backend root; agents with
//! `perSessionKey:false` sign with this persisted key across sequential runs.
//!
//! NOTE: agent keypairs live under `<edge_home>/agents/<slug>.json`, which
//! defaults to `~/.tenex-edge/agents/`. `edge_home()` defaults to `~/.tenex-edge`.

use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod commands;
mod keys;
mod local_agent;
pub(crate) use commands::adapt_argv_for_slug;
pub use commands::{LaunchCommand, DEFAULT_COMMAND_NAME};
pub use keys::{derive_session_keys_v2, SessionIdentity};
pub(crate) use local_agent::add_local_agent_with_commands;
pub use local_agent::{
    add_local_agent, agent_harness_bundle, list_invitable_agents, list_local_agent_details,
    list_local_agents, list_local_pubkeys, remove_local_agent, set_local_agent_byline, LocalAgent,
    SpawnAgentEntry,
};

#[derive(Debug, Serialize, Deserialize)]
struct StoredKey {
    slug: String,
    secret_key: String, // hex
    public_key: String, // hex
    created_at: u64,
    /// Named harness commands to use when spawning a new hosted session for this
    /// agent. The old singular `command` field is intentionally not deserialized:
    /// files that still carry it behave as if no commands are configured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    commands: Vec<LaunchCommand>,
    /// Inline agent definition forwarded to the harness at spawn time.
    /// For Claude: becomes `--agents '{"<slug>": <def>}' --agent <slug>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<serde_json::Value>,
    /// One-line "when to use this agent" note, surfaced in `who`'s agent table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    byline: Option<String>,
    /// When false, this persisted key signs every fresh session for the agent.
    /// The default keeps the normal per-session derived-key contract.
    #[serde(default = "default_per_session_key", rename = "perSessionKey")]
    per_session_key: bool,
    /// Name of the `~/.tenex-edge/harnesses.json` bundle that hosts this agent.
    /// `None` (the default) means the built-in PTY spawn — behavior is unchanged
    /// for every agent that does not opt into a bundle. A bundle whose transport
    /// is `acp`/`app-server` launches the agent over the RPC transport instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    harness: Option<String>,
}

const fn default_per_session_key() -> bool {
    true
}

impl StoredKey {
    /// The byline to display for this agent: the explicit `byline` field,
    /// falling back to the inline agent definition's `description`.
    fn effective_byline(&self) -> Option<String> {
        self.byline
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.agent
                    .as_ref()
                    .and_then(|a| a.get("description"))
                    .and_then(|d| d.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
    }
}

/// A resolved agent identity: its slug, signing keys, and named harness commands.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub slug: String,
    pub keys: Keys,
    pub commands: Vec<LaunchCommand>,
    pub per_session_key: bool,
    /// Configured harness bundle name (see [`StoredKey::harness`]); `None` for the
    /// built-in PTY spawn.
    pub harness: Option<String>,
}

impl AgentIdentity {
    pub fn pubkey_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }
}

fn agents_dir(edge_home: &Path) -> PathBuf {
    edge_home.join("agents")
}

fn key_path(edge_home: &Path, slug: &str) -> PathBuf {
    agents_dir(edge_home).join(format!("{slug}.json"))
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

/// Load the agent's keypair, generating + persisting it on first use.
pub fn load_or_create(edge_home: &Path, slug: &str, now: u64) -> Result<AgentIdentity> {
    load_or_create_with_command(edge_home, slug, now, None)
}

/// Like `load_or_create`, but when the identity doesn't exist yet, persists
/// `command` as its spawn command (e.g. the real argv of a direct `claude
/// --agent <slug>` invocation detected outside `tenex-edge launch`). Ignored
/// — never overwrites an existing identity's stored command.
pub fn load_or_create_with_command(
    edge_home: &Path,
    slug: &str,
    now: u64,
    command: Option<Vec<String>>,
) -> Result<AgentIdentity> {
    validate_slug(slug)?;
    let path = key_path(edge_home, slug);
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading key {}", path.display()))?;
        let stored: StoredKey =
            serde_json::from_str(&s).with_context(|| format!("parsing key {}", path.display()))?;
        let keys = Keys::parse(&stored.secret_key)
            .with_context(|| format!("parsing secret key for {slug}"))?;
        tracing::debug!(slug, pubkey = %&stored.public_key[..8], "agent key loaded");
        return Ok(AgentIdentity {
            slug: slug.to_string(),
            keys,
            commands: commands::normalize_commands(stored.commands),
            per_session_key: stored.per_session_key,
            harness: stored.harness,
        });
    }

    let keys = Keys::generate();
    let commands = command
        .and_then(LaunchCommand::default)
        .into_iter()
        .collect();
    let stored = StoredKey {
        slug: slug.to_string(),
        secret_key: keys.secret_key().to_secret_hex(),
        public_key: keys.public_key().to_hex(),
        created_at: now,
        commands,
        agent: None,
        byline: None,
        per_session_key: true,
        harness: None,
    };
    std::fs::create_dir_all(agents_dir(edge_home))
        .with_context(|| format!("creating {}", agents_dir(edge_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    tracing::info!(slug, pubkey = %&stored.public_key[..8], path = %path.display(), "agent key created");
    Ok(AgentIdentity {
        slug: slug.to_string(),
        keys,
        commands: stored.commands,
        per_session_key: stored.per_session_key,
        harness: stored.harness,
    })
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
