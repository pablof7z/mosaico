use super::super::{
    agents_dir, atomic_write, key_path, normalize_optional_config_name, read_stored_key,
    validate_config_name, validate_slug, AgentIdentity, StoredKey,
};
use anyhow::{Context, Result};
use nostr::Keys;
use std::path::Path;

/// A complete operator-owned launch update plus optional identity/byline changes.
///
/// `None` for `per_session_key` or `byline` preserves the existing value. New
/// agents default to per-session identity and no byline. `Some(None)` clears an
/// existing byline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAgentUpdate {
    pub harness: String,
    pub profile: Option<String>,
    pub per_session_key: Option<bool>,
    pub byline: Option<Option<String>>,
}

/// Create or update a local agent configuration atomically.
///
/// Moving to durable identity generates and persists one matching keypair.
/// Moving back to per-session identity removes both durable key fields. Existing
/// `created_at` and byline values are preserved unless the update says otherwise.
pub fn save_local_agent(
    mosaico_home: &Path,
    slug: &str,
    update: LocalAgentUpdate,
    now: u64,
) -> Result<(AgentIdentity, bool)> {
    validate_slug(slug)?;
    let harness = validate_config_name("harness", &update.harness)?;
    let profile = normalize_optional_config_name("profile", update.profile.as_deref())?;
    let path = key_path(mosaico_home, slug);
    if path.exists() {
        let mut stored = read_stored_key(&path)?;
        stored.identity_keys()?;
        stored.harness = harness;
        stored.profile = profile;
        if let Some(byline) = update.byline {
            stored.byline = normalize_byline(byline);
        }
        let target_per_session_key = update.per_session_key.unwrap_or(stored.per_session_key);
        apply_identity_mode(&mut stored, target_per_session_key);
        atomic_write(&path, &serde_json::to_string_pretty(&stored)?)?;
        return Ok((identity_from_stored(&stored)?, false));
    }

    let mut stored = StoredKey {
        slug: slug.to_string(),
        secret_key: None,
        public_key: None,
        created_at: now,
        byline: update.byline.and_then(normalize_byline),
        per_session_key: update.per_session_key.unwrap_or(true),
        harness,
        profile,
    };
    let target_per_session_key = stored.per_session_key;
    apply_identity_mode(&mut stored, target_per_session_key);
    std::fs::create_dir_all(agents_dir(mosaico_home))
        .with_context(|| format!("creating {}", agents_dir(mosaico_home).display()))?;
    atomic_write(&path, &serde_json::to_string_pretty(&stored)?)?;
    Ok((identity_from_stored(&stored)?, true))
}

fn apply_identity_mode(stored: &mut StoredKey, per_session_key: bool) {
    let was_per_session_key = stored.per_session_key;
    stored.per_session_key = per_session_key;
    if per_session_key {
        stored.secret_key = None;
        stored.public_key = None;
    } else if was_per_session_key || stored.secret_key.is_none() || stored.public_key.is_none() {
        let keys = Keys::generate();
        stored.secret_key = Some(keys.secret_key().to_secret_hex());
        stored.public_key = Some(keys.public_key().to_hex());
    }
}

fn identity_from_stored(stored: &StoredKey) -> Result<AgentIdentity> {
    Ok(AgentIdentity {
        slug: stored.slug.clone(),
        keys: stored.identity_keys()?,
        per_session_key: stored.per_session_key,
        harness: stored.harness.clone(),
        profile: stored.profile.clone(),
    })
}

fn normalize_byline(byline: Option<String>) -> Option<String> {
    byline
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
