//! Agent keystore (M1 §4).
//!
//! `--agent <slug>` resolves to a durable Nostr keypair, generated on first use
//! and persisted under `<edge_home>/agents/<slug>.json`. Identity is
//! `(agent, machine)`: the same slug on another machine is a different key.
//!
//! NOTE: agent keypairs live under `<edge_home>/agents/<slug>.json`, which
//! defaults to `~/.tenex-edge/agents/`. `edge_home()` defaults to `~/.tenex-edge`.

use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod keys;
pub use keys::{
    agent_ordinal_label, derive_agent_ordinal_keys, derive_session_keys, AgentInstance,
};

#[derive(Debug, Serialize, Deserialize)]
struct StoredKey {
    slug: String,
    secret_key: String, // hex
    public_key: String, // hex
    created_at: u64,
    /// Harness command to use when spawning a new tmux session for this agent.
    /// E.g. `["claude", "--dangerously-skip-permissions"]`.
    /// When absent, the spawn logic falls back to the built-in SPAWN_DEFS table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<Vec<String>>,
    /// Inline agent definition forwarded to the harness at spawn time.
    /// For Claude: becomes `--agents '{"<slug>": <def>}' --agent <slug>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<serde_json::Value>,
    /// One-line "when to use this agent" note, surfaced in `who`'s agent table.
    /// Read from `byline` or its alias `useCriteria`.
    #[serde(
        default,
        alias = "useCriteria",
        skip_serializing_if = "Option::is_none"
    )]
    byline: Option<String>,
}

impl StoredKey {
    /// The byline to display for this agent: the explicit `byline`/`useCriteria`
    /// field, falling back to the inline agent definition's `description`.
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

/// A resolved agent identity: its slug, signing keys, and optional harness command.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub slug: String,
    pub keys: Keys,
    /// Harness command from the agent file, if present.
    pub command: Option<Vec<String>>,
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
            command: stored.command,
        });
    }

    let keys = Keys::generate();
    let stored = StoredKey {
        slug: slug.to_string(),
        secret_key: keys.secret_key().to_secret_hex(),
        public_key: keys.public_key().to_hex(),
        created_at: now,
        command: None,
        agent: None,
        byline: None,
    };
    std::fs::create_dir_all(agents_dir(edge_home))
        .with_context(|| format!("creating {}", agents_dir(edge_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    tracing::info!(slug, pubkey = %&stored.public_key[..8], path = %path.display(), "agent key created");
    Ok(AgentIdentity {
        slug: slug.to_string(),
        keys,
        command: None,
    })
}

/// Every agent in the local keystore (their hex pubkeys). Your own fleet trusts
/// itself automatically, so agents on one device see each other without the
/// operator having to pre-whitelist keys that are generated on first use.
pub fn list_local_pubkeys(edge_home: &Path) -> Vec<String> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            // This list is the self-trust whitelist: a key silently skipped here
            // drops an agent's own fleet member out of the trusted set, so a
            // read/parse failure is loud at error! before we skip it.
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<StoredKey>(&s) {
                    Ok(k) => out.push(k.public_key),
                    Err(e) => tracing::error!(
                        path = %path.display(),
                        error = %e,
                        "list_local_pubkeys: skipping corrupt keystore file — agent dropped from self-trust whitelist"
                    ),
                },
                Err(e) => tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "list_local_pubkeys: skipping unreadable keystore file — agent dropped from self-trust whitelist"
                ),
            }
        }
    }
    out
}

/// All agents in the local keystore with their configured harness command (if
/// any) and display byline. Used by the spawn machinery: command from the agent
/// file takes priority over SPAWN_DEFS.
#[allow(clippy::type_complexity)]
pub fn list_local_agents(
    edge_home: &Path,
) -> Vec<(
    String,
    Option<Vec<String>>,
    Option<serde_json::Value>,
    Option<String>,
)> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<StoredKey>(&s) {
                    Ok(k) => {
                        let byline = k.effective_byline();
                        out.push((k.slug, k.command, k.agent, byline));
                    }
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "list_local_agents: skipping corrupt keystore file"
                    ),
                },
                Err(e) => tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "list_local_agents: skipping unreadable keystore file"
                ),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A local agent as listed by `tenex-edge agent list`: its slug, hex pubkey, and
/// optional harness launch command. Distinct from `list_local_agents` (which the
/// spawn path uses) in that it also surfaces the pubkey for the operator.
#[derive(Debug, Clone)]
pub struct LocalAgent {
    pub slug: String,
    pub pubkey: String,
    pub command: Option<Vec<String>>,
}

/// The invitable roster as `(slug, byline, created_at)`, sorted by slug. The
/// `created_at` lets the awareness delta surface only AGENTS THAT BECAME
/// available since a session's last turn (decision D — roster shown on change,
/// not every turn).
pub fn list_invitable_agents(edge_home: &Path) -> Vec<(String, Option<String>, u64)> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<StoredKey>(&s) {
                    Ok(k) => {
                        let byline = k.effective_byline();
                        out.push((k.slug, byline, k.created_at));
                    }
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "list_invitable_agents: skipping corrupt keystore file"
                    ),
                },
                Err(e) => tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "list_invitable_agents: skipping unreadable keystore file"
                ),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Every agent in the local keystore, with slug + pubkey + command, sorted by slug.
pub fn list_local_agent_details(edge_home: &Path) -> Vec<LocalAgent> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<StoredKey>(&s) {
                    Ok(k) => out.push(LocalAgent {
                        slug: k.slug,
                        pubkey: k.public_key,
                        command: k.command,
                    }),
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "list_local_agent_details: skipping corrupt keystore file"
                    ),
                },
                Err(e) => tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "list_local_agent_details: skipping unreadable keystore file"
                ),
            }
        }
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// Add a local agent: mint + persist a keypair if the slug is new. When `command`
/// is `Some`, set (or overwrite) the harness launch command — so this doubles as
/// "set the command for an existing agent". Returns the resolved identity and
/// whether the keypair was newly created (`true`) or already existed (`false`).
pub fn add_local_agent(
    edge_home: &Path,
    slug: &str,
    command: Option<Vec<String>>,
    now: u64,
) -> Result<(AgentIdentity, bool)> {
    validate_slug(slug)?;
    let path = key_path(edge_home, slug);
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading key {}", path.display()))?;
        let mut stored: StoredKey =
            serde_json::from_str(&s).with_context(|| format!("parsing key {}", path.display()))?;
        let keys = Keys::parse(&stored.secret_key)
            .with_context(|| format!("parsing secret key for {slug}"))?;
        if command.is_some() {
            stored.command = command;
            let body = serde_json::to_string_pretty(&stored)?;
            atomic_write(&path, &body)?;
        }
        return Ok((
            AgentIdentity {
                slug: slug.to_string(),
                keys,
                command: stored.command,
            },
            false,
        ));
    }

    let keys = Keys::generate();
    let stored = StoredKey {
        slug: slug.to_string(),
        secret_key: keys.secret_key().to_secret_hex(),
        public_key: keys.public_key().to_hex(),
        created_at: now,
        command: command.clone(),
        agent: None,
        byline: None,
    };
    std::fs::create_dir_all(agents_dir(edge_home))
        .with_context(|| format!("creating {}", agents_dir(edge_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    Ok((
        AgentIdentity {
            slug: slug.to_string(),
            keys,
            command,
        },
        true,
    ))
}

/// Remove a local agent by soft-deleting its keystore file: the private key is
/// renamed to `<slug>.json.removed` rather than unlinked, so a mistaken removal
/// is recoverable with a single `mv` (a freshly minted key would otherwise be a
/// *different* identity, losing the agent's pubkey forever). Returns the path the
/// key was parked at, or `None` if no such agent existed.
pub fn remove_local_agent(edge_home: &Path, slug: &str) -> Result<Option<PathBuf>> {
    validate_slug(slug)?;
    let path = key_path(edge_home, slug);
    if !path.exists() {
        return Ok(None);
    }
    let parked = path.with_extension("json.removed");
    std::fs::rename(&path, &parked)
        .with_context(|| format!("parking {} -> {}", path.display(), parked.display()))?;
    Ok(Some(parked))
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
