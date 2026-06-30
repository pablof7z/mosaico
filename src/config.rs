//! Device-level config + tenex-edge's own writable home.
//!
//! tenex-edge *reads* `~/.tenex-edge/config.json` (for `whitelistedPubkeys`,
//! optional `relays`, and `backendName` as the host label) and keeps all of its
//! own writable state under `~/.tenex-edge`.

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
    /// Human operator's Nostr secret key (bech32 nsec or hex). Used for exactly
    /// one purpose: signing user-prompt events when the human submits a prompt
    /// from the CLI. The operator's pubkey is NOT derived from this field for
    /// group admin grants — the operator's pubkey lives in `whitelisted_pubkeys`
    /// (config `whitelistedPubkeys`), which is the source of truth for who is an
    /// admin in every project group. Never used for group management,
    /// session-key derivation, or backend identity.
    pub user_nsec: Option<String>,
    /// This backend/daemon's own Nostr secret key (bech32 nsec or hex). The
    /// sole signer for NIP-29 group management, session-key derivation, and
    /// backend identity. Its pubkey is added as an admin to every group we
    /// create and is the address the orchestration listener matches `add`
    /// tags against.
    pub tenex_private_key: Option<String>,
    /// Custom tmux status-format string. None means use the default.
    pub tmux_status_command: Option<String>,
    /// Whether human-initiated sessions (no `TENEX_EDGE_CHANNEL` override) mint
    /// their own per-session NIP-29 subgroup. Default `false`: such sessions
    /// land in the bare project channel, and `tenex-edge launch` (without
    /// `--channel`) opens the interactive channel picker instead of minting.
    /// When `true`, the legacy behavior is restored (mint a per-session room).
    pub per_session_rooms: bool,
}

impl Config {
    /// Key used as the HKDF IKM for per-session key derivation. The backend's
    /// own key (`tenexPrivateKey`) — never the operator's `userNsec`.
    pub fn session_ikm_nsec(&self) -> Option<&String> {
        self.tenex_private_key.as_ref()
    }

    /// Signer for NIP-29 group-management events (create/lock/put-user/
    /// put-admin/remove-user/edit-metadata). Always the backend's own
    /// `tenexPrivateKey` — the operator's `userNsec` is no longer used for
    /// group management. The operator's pubkey is instead *granted* the admin
    /// role by this signer (see `Nip29Provider::open_project`).
    pub fn management_nsec(&self) -> Option<&String> {
        self.tenex_private_key.as_ref()
    }

    /// This backend's own identity key. Always `tenexPrivateKey`; there is no
    /// fallback to `userNsec` — the operator key is a human identity, not a
    /// backend identity.
    pub fn backend_nsec(&self) -> Option<&String> {
        self.tenex_private_key.as_ref()
    }

    /// The human operator's Nostr secret key. Used in exactly one place:
    /// `rpc_user_prompt` signs the user's prompt as the operator. The
    /// operator's pubkey is NOT derived from this field for group admin grants —
    /// it lives in `whitelisted_pubkeys` instead. Never used for group
    /// management, session-key derivation, or backend identity.
    pub fn user_nsec(&self) -> Option<&String> {
        self.user_nsec.as_ref()
    }
}

/// Mirror of the relevant fields in `~/.tenex-edge/config.json`. Unknown fields are
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
    /// Backend's own signing key for group management, session derivation, and
    /// backend identity.
    #[serde(default, rename = "tenexPrivateKey")]
    tenex_private_key: Option<String>,
    /// Custom tmux status-format string for agent sessions. When set, overrides
    /// the default `tenex-edge statusline` command. Use tmux format variables
    /// `#{q:@te_session}` (the canonical session id, stamped by the daemon once
    /// the session-start hook fires), `#{@te_agent}`, and `#{q:@te_cwd}` to
    /// reference the session's identity. `#{q:@te_session}` is the preferred key:
    /// it disambiguates panes of the same agent in the same project; the others
    /// are fallbacks for the brief window before the hook fires.
    #[serde(default, rename = "tmuxStatusCommand")]
    tmux_status_command: Option<String>,
    /// Opt-in: mint a per-session subgroup for human-initiated sessions.
    /// Defaults to `false` (use the project channel; `launch` opens the picker).
    #[serde(default, rename = "perSessionRooms")]
    per_session_rooms: bool,
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
            user_nsec: raw.user_nsec,
            tenex_private_key: raw.tenex_private_key,
            tmux_status_command: raw.tmux_status_command,
            per_session_rooms: raw.per_session_rooms,
        })
    }

    /// Load from `~/.tenex-edge/config.json` (or `$TENEX_CONFIG` override).
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
    edge_home().join("config.json")
}

/// tenex-edge's own writable root. Override with `$TENEX_EDGE_HOME` (tests use
/// this for isolation). Default: `~/.tenex-edge`.
pub fn edge_home() -> PathBuf {
    if let Ok(p) = std::env::var("TENEX_EDGE_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".tenex-edge")
}


pub fn ensure_dir(p: &Path) -> Result<()> {
    std::fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
    Ok(())
}

fn home_dir() -> PathBuf {
    // Identity-root invariant. `edge_home` checks TENEX_EDGE_HOME first, so we
    // only reach here when that override is absent. With HOME ALSO unset (a
    // stripped launchd/cron env), defaulting to "." would relocate the keystore,
    // config, and state.db under ./.tenex-edge — agents would mint NEW identities
    // and the trust whitelist would empty. Refuse to run rather than guess.
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .expect(
            "neither TENEX_EDGE_HOME nor HOME is set: refusing to relocate keystore/config/state.db \
             under ./.tenex-edge (would mint new agent identities and empty the trust whitelist)",
        );
    PathBuf::from(home)
}

pub fn hostname() -> String {
    let resolved = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match resolved {
        Some(h) => h,
        None => {
            // The hostname feeds the backend identity component; sharing a
            // sentinel silently would let multiple hosts collide under one name.
            tracing::warn!(
                "hostname(): could not resolve system hostname — falling back to \"unknown-host\" \
                 (set backendName to avoid an identity collision)"
            );
            "unknown-host".to_string()
        }
    }
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
        assert_eq!(c.tenex_private_key.as_deref(), Some("deadbeef"));
        assert_eq!(c.session_ikm_nsec().map(String::as_str), Some("deadbeef"));
        assert_eq!(c.management_nsec().map(String::as_str), Some("deadbeef"));
        assert_eq!(c.backend_nsec().map(String::as_str), Some("deadbeef"));
        assert!(c.user_nsec().is_none());
    }

    #[test]
    fn key_accessors_split_when_both_present() {
        let json = r#"{
            "whitelistedPubkeys": [],
            "userNsec": "operatorkey",
            "tenexPrivateKey": "backendkey"
        }"#;
        let c = Config::from_json_str(json, "host").unwrap();
        // session derivation + management + backend identity all use the
        // backend key; the operator key is only for user prompts + admin grant.
        assert_eq!(c.session_ikm_nsec().map(String::as_str), Some("backendkey"));
        assert_eq!(c.management_nsec().map(String::as_str), Some("backendkey"));
        assert_eq!(c.backend_nsec().map(String::as_str), Some("backendkey"));
        assert_eq!(c.user_nsec().map(String::as_str), Some("operatorkey"));
    }

    #[test]
    fn user_nsec_alone_is_not_a_management_key() {
        let json = r#"{ "userNsec": "operatorkey" }"#;
        let c = Config::from_json_str(json, "host").unwrap();
        // No tenexPrivateKey → no management, session derivation, or backend.
        assert!(c.management_nsec().is_none());
        assert!(c.session_ikm_nsec().is_none());
        assert!(c.backend_nsec().is_none());
        // The operator key is still available for user prompts + admin grant.
        assert_eq!(c.user_nsec().map(String::as_str), Some("operatorkey"));
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
    fn per_session_rooms_defaults_off_and_parses_when_set() {
        let off = Config::from_json_str(r#"{"whitelistedPubkeys":[]}"#, "host").unwrap();
        assert!(!off.per_session_rooms);
        let on = Config::from_json_str(
            r#"{"whitelistedPubkeys":[],"perSessionRooms":true}"#,
            "host",
        )
        .unwrap();
        assert!(on.per_session_rooms);
    }

    #[test]
    fn edge_home_honors_override() {
        std::env::set_var("TENEX_EDGE_HOME", "/tmp/te-test-home");
        assert_eq!(edge_home(), PathBuf::from("/tmp/te-test-home"));
        std::env::remove_var("TENEX_EDGE_HOME");
    }
}
