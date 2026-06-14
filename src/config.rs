//! Device-level config + tenex-edge's own writable home.
//!
//! tenex-edge *reads* the shared `~/.tenex/config.json` (for `whitelistedPubkeys`,
//! optional `relays`, and `backendName` as the host label) but keeps all of its
//! own writable state under a SEPARATE home so it never clobbers TENEX/pc data
//! that already lives in `~/.tenex/agents`, `~/.tenex/data`, etc.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const DEFAULT_RELAY: &str = "wss://nip29.f7z.io";
pub const DEFAULT_INDEXER_RELAY: &str = "wss://purplepag.es";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub whitelisted_pubkeys: Vec<String>,
    pub relays: Vec<String>,
    /// Indexer relay for kind:0 profile discovery (default: purplepag.es).
    /// Receives all kind:0 publishes and is queried for profile lookups.
    pub indexer_relay: String,
    /// Host label published on the agent's profile (M1 §3 `host` tag).
    pub host: String,
    /// Human user's Nostr secret key (bech32 nsec or hex). Used to publish
    /// user-prompt events on behalf of the human, not the agent.
    pub user_nsec: Option<String>,
}

/// Mirror of the relevant fields in `~/.tenex/config.json`. Unknown fields are
/// ignored, so we coexist with TENEX's much larger (camelCase) config.
#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default, rename = "whitelistedPubkeys")]
    whitelisted_pubkeys: Vec<String>,
    #[serde(default)]
    relays: Vec<String>,
    /// Indexer relay for kind:0 profile publishing and lookup. Defaults to purplepag.es.
    #[serde(default, rename = "indexerRelay")]
    indexer_relay: Option<String>,
    #[serde(default, rename = "backendName")]
    backend_name: Option<String>,
    #[serde(default, rename = "userNsec")]
    user_nsec: Option<String>,
    /// Fallback signing key for NIP-29 group management when userNsec is absent.
    #[serde(default, rename = "tenexPrivateKey")]
    tenex_private_key: Option<String>,
}

impl Config {
    /// Parse from a JSON string. Pure — the unit-testable core of `load`.
    pub fn from_json_str(s: &str, fallback_host: &str) -> Result<Self> {
        let raw: RawConfig = serde_json::from_str(s).context("parsing tenex config json")?;
        let relays = if raw.relays.is_empty() {
            vec![DEFAULT_RELAY.to_string()]
        } else {
            raw.relays
        };
        let host = raw
            .backend_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| fallback_host.to_string());
        let indexer_relay = raw
            .indexer_relay
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_INDEXER_RELAY.to_string());
        Ok(Config {
            whitelisted_pubkeys: raw.whitelisted_pubkeys,
            relays,
            indexer_relay,
            host,
            user_nsec: raw.user_nsec.or(raw.tenex_private_key),
        })
    }

    /// Load from `~/.tenex/config.json` (or `$TENEX_CONFIG` override).
    pub fn load() -> Result<Self> {
        let path = config_path();
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::from_json_str(&s, &hostname())
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("TENEX_CONFIG") {
        return PathBuf::from(p);
    }
    home_dir().join(".tenex").join("config.json")
}

/// tenex-edge's own writable root. Override with `$TENEX_EDGE_HOME` (tests use
/// this for isolation). Default: `~/.tenex/edge`.
pub fn edge_home() -> PathBuf {
    if let Ok(p) = std::env::var("TENEX_EDGE_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".tenex").join("edge")
}

/// The shared `~/.tenex` directory (override with `$TENEX_DIR`, for tests).
pub fn tenex_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TENEX_DIR") {
        return PathBuf::from(p);
    }
    home_dir().join(".tenex")
}

pub fn ensure_dir(p: &Path) -> Result<()> {
    std::fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
    Ok(())
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_tenex_shape_with_camelcase() {
        let json = r#"{
            "version": 3,
            "whitelistedPubkeys": ["aa", "bb"],
            "backendName": "pablos' laptop",
            "tenexPrivateKey": "deadbeef"
        }"#;
        let c = Config::from_json_str(json, "fallback").unwrap();
        assert_eq!(c.whitelisted_pubkeys, vec!["aa", "bb"]);
        assert_eq!(c.host, "pablos' laptop");
        assert_eq!(c.relays, vec![DEFAULT_RELAY]); // defaulted
        assert_eq!(c.indexer_relay, DEFAULT_INDEXER_RELAY); // defaulted
    }

    #[test]
    fn explicit_relays_win_and_host_falls_back() {
        let json = r#"{"whitelistedPubkeys":[],"relays":["wss://r1","wss://r2"]}"#;
        let c = Config::from_json_str(json, "fallback-host").unwrap();
        assert_eq!(c.relays, vec!["wss://r1", "wss://r2"]);
        assert_eq!(c.host, "fallback-host");
        assert!(c.whitelisted_pubkeys.is_empty());
        assert_eq!(c.indexer_relay, DEFAULT_INDEXER_RELAY);
    }

    #[test]
    fn custom_indexer_relay() {
        let json = r#"{"indexerRelay":"wss://my-indexer.example"}"#;
        let c = Config::from_json_str(json, "host").unwrap();
        assert_eq!(c.indexer_relay, "wss://my-indexer.example");
    }

    #[test]
    fn edge_home_honors_override() {
        std::env::set_var("TENEX_EDGE_HOME", "/tmp/te-test-home");
        assert_eq!(edge_home(), PathBuf::from("/tmp/te-test-home"));
        std::env::remove_var("TENEX_EDGE_HOME");
    }
}
