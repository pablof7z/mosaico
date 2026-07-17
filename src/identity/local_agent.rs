use super::{agents_dir, atomic_write, key_path, read_stored_key, validate_slug, AgentIdentity};
use anyhow::{bail, Context, Result};
use std::path::Path;

mod save;
pub use save::{save_local_agent, LocalAgentUpdate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunchConfig {
    pub harness: String,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeystoreEntry {
    pub(crate) slug: String,
    pub(crate) pubkey: Option<String>,
    pub(crate) created_at: u64,
    pub(crate) per_session_key: bool,
    pub(crate) harness: String,
    pub(crate) profile: Option<String>,
    pub(crate) byline: Option<String>,
}

/// The sole directory reader for the local agent keystore.
pub(crate) fn keystore_entries(mosaico_home: &Path) -> Vec<KeystoreEntry> {
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
            match read_stored_key(&path) {
                Ok(k) => {
                    let byline = k.effective_byline();
                    out.push(KeystoreEntry {
                        slug: k.slug,
                        pubkey: k.public_key,
                        created_at: k.created_at,
                        per_session_key: k.per_session_key,
                        harness: k.harness,
                        profile: k.profile,
                        byline,
                    })
                }
                Err(e) => tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "keystore_entries: skipping corrupt agent file"
                ),
            }
        }
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// Every durable agent pubkey trusted as part of this machine's own fleet.
pub fn list_local_pubkeys(mosaico_home: &Path) -> Vec<String> {
    keystore_entries(mosaico_home)
        .into_iter()
        .filter(|entry| !entry.per_session_key)
        .filter_map(|entry| entry.pubkey)
        .collect()
}

/// All configured agents as `(slug, harness, profile, byline)`.
pub fn list_local_agents(
    mosaico_home: &Path,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    keystore_entries(mosaico_home)
        .into_iter()
        .map(|entry| (entry.slug, entry.harness, entry.profile, entry.byline))
        .collect()
}

/// The invitable roster as `(slug, byline, created_at)`, sorted by slug. The
/// `created_at` lets the awareness delta surface only agents that became
/// available since a session's last turn.
pub fn list_invitable_agents(mosaico_home: &Path) -> Vec<(String, Option<String>, u64)> {
    keystore_entries(mosaico_home)
        .into_iter()
        .map(|entry| (entry.slug, entry.byline, entry.created_at))
        .collect()
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

/// Add or update a configured local agent. The harness bundle is required; an
/// absent profile means to use the harness-native default.
pub fn add_local_agent(
    mosaico_home: &Path,
    slug: &str,
    harness: &str,
    profile: Option<&str>,
    now: u64,
) -> Result<(AgentIdentity, bool)> {
    save_local_agent(
        mosaico_home,
        slug,
        LocalAgentUpdate {
            harness: harness.to_string(),
            profile: profile.map(str::to_string),
            per_session_key: None,
            byline: None,
        },
        now,
    )
}

/// Load the exact bundle/profile selection for a configured agent.
pub fn agent_launch_config(mosaico_home: &Path, slug: &str) -> Result<AgentLaunchConfig> {
    let path = key_path(mosaico_home, slug);
    let stored = read_stored_key(&path)?;
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
    let mut stored = read_stored_key(&path)?;
    stored.byline = byline
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    Ok(())
}
/// Permanently delete the configured agent file.
pub fn remove_local_agent(mosaico_home: &Path, slug: &str) -> Result<bool> {
    validate_slug(slug)?;
    let path = key_path(mosaico_home, slug);
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
    Ok(true)
}
