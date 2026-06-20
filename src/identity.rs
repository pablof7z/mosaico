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
    #[serde(default, alias = "useCriteria", skip_serializing_if = "Option::is_none")]
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

/// All agents in the local keystore with their configured harness command (if
/// any) and display byline. Used by the spawn machinery: command from the agent
/// file takes priority over SPAWN_DEFS.
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
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(k) = serde_json::from_str::<StoredKey>(&s) {
                    let byline = k.effective_byline();
                    out.push((k.slug, k.command, k.agent, byline));
                }
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
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(k) = serde_json::from_str::<StoredKey>(&s) {
                    out.push(LocalAgent {
                        slug: k.slug,
                        pubkey: k.public_key,
                        command: k.command,
                    });
                }
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

    #[test]
    fn add_local_agent_creates_then_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let (a, created) = add_local_agent(dir.path(), "coder", None, 1).unwrap();
        assert!(created, "first add mints a fresh key");
        assert!(dir.path().join("agents").join("coder.json").exists());

        // Second add with no command returns the SAME key, created=false.
        let (b, created2) = add_local_agent(dir.path(), "coder", None, 2).unwrap();
        assert!(!created2, "re-adding an existing slug does not recreate");
        assert_eq!(a.pubkey_hex(), b.pubkey_hex());
    }

    #[test]
    fn add_local_agent_sets_and_overwrites_command() {
        let dir = tempfile::tempdir().unwrap();
        // Create with a command.
        let (a, _) = add_local_agent(
            dir.path(),
            "dev",
            Some(vec![
                "claude".into(),
                "--dangerously-skip-permissions".into(),
            ]),
            1,
        )
        .unwrap();
        assert_eq!(
            a.command.as_deref().unwrap(),
            &["claude", "--dangerously-skip-permissions"]
        );
        // Overwrite the command on the existing agent; key is unchanged.
        let (b, created) =
            add_local_agent(dir.path(), "dev", Some(vec!["codex".into()]), 2).unwrap();
        assert!(!created);
        assert_eq!(a.pubkey_hex(), b.pubkey_hex());
        assert_eq!(b.command.as_deref().unwrap(), &["codex"]);
    }

    #[test]
    fn remove_local_agent_parks_then_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path(), "coder", 1).unwrap();
        let live = dir.path().join("agents").join("coder.json");
        assert!(live.exists());

        let parked = remove_local_agent(dir.path(), "coder").unwrap();
        let parked = parked.expect("removing an existing agent returns the parked path");
        assert!(!live.exists(), "live key file is gone");
        assert!(parked.exists(), "key is parked, not unlinked");
        // Parked file is not a `.json`, so it drops out of the listings.
        assert!(list_local_agent_details(dir.path()).is_empty());
        assert!(list_local_pubkeys(dir.path()).is_empty());

        // Removing again is a no-op (None), not an error.
        assert!(remove_local_agent(dir.path(), "coder").unwrap().is_none());
    }

    #[test]
    fn list_local_agent_details_surfaces_pubkey_and_command() {
        let dir = tempfile::tempdir().unwrap();
        let (a, _) = add_local_agent(dir.path(), "coder", None, 1).unwrap();
        add_local_agent(dir.path(), "dev", Some(vec!["codex".into()]), 1).unwrap();
        let rows = list_local_agent_details(dir.path());
        assert_eq!(rows.len(), 2);
        // Sorted by slug: coder, dev.
        assert_eq!(rows[0].slug, "coder");
        assert_eq!(rows[0].pubkey, a.pubkey_hex());
        assert!(rows[0].command.is_none());
        assert_eq!(rows[1].slug, "dev");
        assert_eq!(rows[1].command.as_deref().unwrap(), &["codex"]);
    }

    #[test]
    fn command_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        // Write a file with a command field manually
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        std::fs::write(
            dir.path().join("agents/dev.json"),
            r#"{"slug":"dev","secret_key":"0000000000000000000000000000000000000000000000000000000000000001","public_key":"","created_at":1,"command":["claude","--dangerously-skip-permissions"]}"#,
        )
        .unwrap();
        let agents = list_local_agents(dir.path());
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].0, "dev");
        assert_eq!(
            agents[0].1.as_deref().unwrap(),
            &["claude", "--dangerously-skip-permissions"]
        );
        assert!(agents[0].2.is_none());
        assert!(agents[0].3.is_none());
    }

    #[test]
    fn byline_reads_field_alias_and_falls_back_to_agent_description() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        // Explicit `byline`.
        std::fs::write(
            dir.path().join("agents/a.json"),
            r#"{"slug":"a","secret_key":"0000000000000000000000000000000000000000000000000000000000000001","public_key":"","created_at":1,"byline":"front-line triage"}"#,
        )
        .unwrap();
        // `useCriteria` alias.
        std::fs::write(
            dir.path().join("agents/b.json"),
            r#"{"slug":"b","secret_key":"0000000000000000000000000000000000000000000000000000000000000002","public_key":"","created_at":1,"useCriteria":"use for deep research"}"#,
        )
        .unwrap();
        // Falls back to the inline agent definition's `description`.
        std::fs::write(
            dir.path().join("agents/c.json"),
            r#"{"slug":"c","secret_key":"0000000000000000000000000000000000000000000000000000000000000003","public_key":"","created_at":1,"agent":{"description":"writes social posts"}}"#,
        )
        .unwrap();
        // No byline anywhere.
        std::fs::write(
            dir.path().join("agents/d.json"),
            r#"{"slug":"d","secret_key":"0000000000000000000000000000000000000000000000000000000000000004","public_key":"","created_at":1}"#,
        )
        .unwrap();

        let agents = list_local_agents(dir.path());
        let byline = |slug: &str| {
            agents
                .iter()
                .find(|a| a.0 == slug)
                .and_then(|a| a.3.clone())
        };
        assert_eq!(byline("a").as_deref(), Some("front-line triage"));
        assert_eq!(byline("b").as_deref(), Some("use for deep research"));
        assert_eq!(byline("c").as_deref(), Some("writes social posts"));
        assert_eq!(byline("d"), None);
    }
}
