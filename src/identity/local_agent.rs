use super::{
    agents_dir, atomic_write, commands, key_path, validate_slug, AgentIdentity, LaunchCommand,
    StoredKey,
};
use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::*;
use std::path::{Path, PathBuf};

pub type SpawnAgentEntry = (
    String,
    Vec<LaunchCommand>,
    Option<serde_json::Value>,
    Option<String>,
);

/// A local agent as listed by `tenex-edge mgmt agent list`: its slug, hex pubkey, and
/// configured harness launch commands. Distinct from `list_local_agents` (which
/// the spawn path uses) in that it also surfaces the pubkey for the operator.
#[derive(Debug, Clone)]
pub struct LocalAgent {
    pub slug: String,
    pub pubkey: String,
    pub commands: Vec<LaunchCommand>,
    pub per_session_key: bool,
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

/// All agents in the local keystore with their configured harness commands and
/// display byline. Used by the spawn machinery: commands from the agent file
/// take priority over SPAWN_DEFS.
pub fn list_local_agents(edge_home: &Path) -> Vec<SpawnAgentEntry> {
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
                        out.push((
                            k.slug,
                            commands::normalize_commands(k.commands),
                            k.agent,
                            byline,
                        ));
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

/// The invitable roster as `(slug, byline, created_at)`, sorted by slug. The
/// `created_at` lets the awareness delta surface only agents that became
/// available since a session's last turn.
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

/// Every agent in the local keystore, with slug + pubkey + commands, sorted by slug.
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
                        commands: commands::normalize_commands(k.commands),
                        per_session_key: k.per_session_key,
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

/// Add a local agent: mint + persist a keypair if the slug is new. When
/// `command` is `Some`, set (or overwrite) the default named harness launch
/// command. Returns the resolved identity and whether the keypair was newly
/// created (`true`) or already existed (`false`).
pub fn add_local_agent(
    edge_home: &Path,
    slug: &str,
    command: Option<Vec<String>>,
    now: u64,
) -> Result<(AgentIdentity, bool)> {
    let commands = command
        .and_then(LaunchCommand::default)
        .into_iter()
        .collect();
    add_local_agent_with_commands(edge_home, slug, commands, now)
}

pub(crate) fn add_local_agent_with_commands(
    edge_home: &Path,
    slug: &str,
    commands: Vec<LaunchCommand>,
    now: u64,
) -> Result<(AgentIdentity, bool)> {
    validate_slug(slug)?;
    let commands = commands::normalize_commands(commands);
    let path = key_path(edge_home, slug);
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading key {}", path.display()))?;
        let mut stored: StoredKey =
            serde_json::from_str(&s).with_context(|| format!("parsing key {}", path.display()))?;
        let keys = Keys::parse(&stored.secret_key)
            .with_context(|| format!("parsing secret key for {slug}"))?;
        if !commands.is_empty() {
            stored.commands = commands;
            let body = serde_json::to_string_pretty(&stored)?;
            atomic_write(&path, &body)?;
        }
        let commands = commands::normalize_commands(stored.commands);
        return Ok((
            AgentIdentity {
                slug: slug.to_string(),
                keys,
                commands,
                per_session_key: stored.per_session_key,
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
        commands: commands.clone(),
        agent: None,
        byline: None,
        per_session_key: true,
    };
    std::fs::create_dir_all(agents_dir(edge_home))
        .with_context(|| format!("creating {}", agents_dir(edge_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    Ok((
        AgentIdentity {
            slug: slug.to_string(),
            keys,
            commands,
            per_session_key: stored.per_session_key,
        },
        true,
    ))
}

/// Set the local "when to use this agent" byline for an existing agent.
pub fn set_local_agent_byline(edge_home: &Path, slug: &str, byline: Option<String>) -> Result<()> {
    validate_slug(slug)?;
    let path = key_path(edge_home, slug);
    if !path.exists() {
        bail!("no such local agent: {slug}");
    }
    let s = std::fs::read_to_string(&path)
        .with_context(|| format!("reading key {}", path.display()))?;
    let mut stored: StoredKey =
        serde_json::from_str(&s).with_context(|| format!("parsing key {}", path.display()))?;
    stored.byline = byline
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    Ok(())
}

/// Soft-delete the keystore file so a mistaken removal is recoverable.
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
