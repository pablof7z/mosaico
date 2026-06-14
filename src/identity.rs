//! Agent keystore (M1 §4).
//!
//! `--agent <slug>` resolves to a durable Nostr keypair, generated on first use
//! and persisted under `<edge_home>/agents/<slug>.json`. Identity is
//! `(agent, machine)`: the same slug on another machine is a different key.
//!
//! NOTE: this is a SEPARATE directory from TENEX's `~/.tenex/agents` — we never
//! touch those. `edge_home()` defaults to `~/.tenex/edge`.

use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
struct StoredKey {
    slug: String,
    secret_key: String, // hex
    public_key: String, // hex
    created_at: u64,
}

/// A resolved agent identity: its slug + signing keys.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub slug: String,
    pub keys: Keys,
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
        return Ok(AgentIdentity {
            slug: slug.to_string(),
            keys,
        });
    }

    let keys = Keys::generate();
    let stored = StoredKey {
        slug: slug.to_string(),
        secret_key: keys.secret_key().to_secret_hex(),
        public_key: keys.public_key().to_hex(),
        created_at: now,
    };
    std::fs::create_dir_all(agents_dir(edge_home))
        .with_context(|| format!("creating {}", agents_dir(edge_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    Ok(AgentIdentity {
        slug: slug.to_string(),
        keys,
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
            if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Ok(s) = std::fs::read_to_string(e.path()) {
                if let Ok(k) = serde_json::from_str::<StoredKey>(&s) {
                    out.push(k.public_key);
                }
            }
        }
    }
    out
}

/// Slugs of every agent in the local keystore. Used to populate the
/// "spawnable" list in the TUI — agents tenex-edge already knows about
/// and can start a new session for.
pub fn list_local_slugs(edge_home: &Path) -> Vec<String> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|x| x.to_str()) {
                out.push(stem.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Write via a temp file + rename so a crash never leaves a half-written key.
fn atomic_write(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_then_reloads_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_create(dir.path(), "coder", 100).unwrap();
        let b = load_or_create(dir.path(), "coder", 200).unwrap();
        assert_eq!(a.pubkey_hex(), b.pubkey_hex());
        assert_eq!(
            a.keys.secret_key().to_secret_hex(),
            b.keys.secret_key().to_secret_hex()
        );
    }

    #[test]
    fn distinct_slugs_get_distinct_keys() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_create(dir.path(), "coder", 1).unwrap();
        let b = load_or_create(dir.path(), "reviewer", 1).unwrap();
        assert_ne!(a.pubkey_hex(), b.pubkey_hex());
    }

    #[test]
    fn rejects_bad_slug() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_or_create(dir.path(), "bad slug/with-stuff", 1).is_err());
        assert!(load_or_create(dir.path(), "", 1).is_err());
    }

    #[test]
    fn persists_to_expected_path() {
        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path(), "coder", 1).unwrap();
        assert!(dir.path().join("agents").join("coder.json").exists());
    }
}
