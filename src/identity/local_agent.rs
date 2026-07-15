use super::{
    agents_dir, atomic_write, key_path, normalize_optional_config_name, validate_config_name,
    validate_slug, AgentIdentity, StoredKey,
};
use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::*;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunchConfig {
    pub harness: String,
    pub profile: Option<String>,
}

/// A local agent as listed by `mosaico mgmt agent list`: its slug, hex pubkey, and
/// configured launch selection. Distinct from `list_local_agents` (which the
/// roster path uses) in that it also surfaces the pubkey for the operator.
#[derive(Debug, Clone)]
pub struct LocalAgent {
    pub slug: String,
    pub pubkey: String,
    pub per_session_key: bool,
    pub harness: String,
    pub profile: Option<String>,
}

/// Every agent in the local keystore (their hex pubkeys). Your own fleet trusts
/// itself automatically, so agents on one device see each other without the
/// operator having to pre-whitelist keys that are generated on first use.
pub fn list_local_pubkeys(mosaico_home: &Path) -> Vec<String> {
    let dir = agents_dir(mosaico_home);
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

/// All configured agents as `(slug, harness, profile, byline)`.
pub fn list_local_agents(
    mosaico_home: &Path,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let dir = agents_dir(mosaico_home);
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
                        out.push((k.slug, k.harness, k.profile, byline));
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
pub fn list_invitable_agents(mosaico_home: &Path) -> Vec<(String, Option<String>, u64)> {
    let dir = agents_dir(mosaico_home);
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

/// `(slug, effective_byline_or_empty)` for every local agent, sorted by slug —
/// the exact set advertised to clients. Both the backend kind:0 `agent` tags and
/// the kind:30555 roster are built from this, so an add-agent picker's slug
/// round-trips through the `add <slug>` management command.
pub fn list_advertised_agents(mosaico_home: &Path) -> Vec<(String, String)> {
    list_local_agents(mosaico_home)
        .into_iter()
        .map(|(slug, _harness, _profile, byline)| (slug, byline.unwrap_or_default()))
        .collect()
}

/// Every agent in the local keystore, sorted by slug.
pub fn list_local_agent_details(mosaico_home: &Path) -> Vec<LocalAgent> {
    let dir = agents_dir(mosaico_home);
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
                        per_session_key: k.per_session_key,
                        harness: k.harness,
                        profile: k.profile,
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

/// Add or update a configured local agent. The harness bundle is required; an
/// absent profile means to use the harness-native default.
pub fn add_local_agent(
    mosaico_home: &Path,
    slug: &str,
    harness: &str,
    profile: Option<&str>,
    now: u64,
) -> Result<(AgentIdentity, bool)> {
    validate_slug(slug)?;
    let harness = validate_config_name("harness", harness)?;
    let profile = normalize_optional_config_name("profile", profile)?;
    let path = key_path(mosaico_home, slug);
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading key {}", path.display()))?;
        let mut stored: StoredKey =
            serde_json::from_str(&s).with_context(|| format!("parsing key {}", path.display()))?;
        let keys = Keys::parse(&stored.secret_key)
            .with_context(|| format!("parsing secret key for {slug}"))?;
        stored.harness = harness;
        stored.profile = profile;
        let body = serde_json::to_string_pretty(&stored)?;
        atomic_write(&path, &body)?;
        return Ok((
            AgentIdentity {
                slug: slug.to_string(),
                keys,
                per_session_key: stored.per_session_key,
                harness: stored.harness,
                profile: stored.profile,
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
        byline: None,
        per_session_key: true,
        harness,
        profile,
    };
    std::fs::create_dir_all(agents_dir(mosaico_home))
        .with_context(|| format!("creating {}", agents_dir(mosaico_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    Ok((
        AgentIdentity {
            slug: slug.to_string(),
            keys,
            per_session_key: stored.per_session_key,
            harness: stored.harness,
            profile: stored.profile,
        },
        true,
    ))
}

/// Load the exact bundle/profile selection for a configured agent.
pub fn agent_launch_config(mosaico_home: &Path, slug: &str) -> Result<AgentLaunchConfig> {
    let path = key_path(mosaico_home, slug);
    let s = std::fs::read_to_string(&path)
        .with_context(|| format!("reading configured agent {}", path.display()))?;
    let stored: StoredKey =
        serde_json::from_str(&s).with_context(|| format!("parsing agent {}", path.display()))?;
    Ok(AgentLaunchConfig {
        harness: stored.harness,
        profile: stored.profile,
    })
}

/// Set the local "when to use this agent" byline for an existing agent.
pub fn set_local_agent_byline(home: &Path, slug: &str, byline: Option<String>) -> Result<()> {
    validate_slug(slug)?;
    let path = key_path(home, slug);
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
pub fn remove_local_agent(mosaico_home: &Path, slug: &str) -> Result<Option<PathBuf>> {
    validate_slug(slug)?;
    let path = key_path(mosaico_home, slug);
    if !path.exists() {
        return Ok(None);
    }
    let parked = path.with_extension("json.removed");
    std::fs::rename(&path, &parked)
        .with_context(|| format!("parking {} -> {}", path.display(), parked.display()))?;
    Ok(Some(parked))
}
