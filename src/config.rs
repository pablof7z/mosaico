//! Device-level config + tenex-edge's own writable home.
//!
//! tenex-edge *reads* `~/.tenex-edge/config.json` (for `whitelistedPubkeys`,
//! optional `relays`, and `backendName` as the host label) and keeps all of its
//! own writable state under `~/.tenex-edge`.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const DEFAULT_RELAY: &str = "wss://nip29.f7z.io";
pub const DEFAULT_INDEXER_RELAY: &str = "wss://purplepag.es";
pub const ISOLATED_HOME_ACK_ENV: &str = "TENEX_EDGE_ISOLATED_HOME_OK";
const MISSING_HOME_MESSAGE: &str =
    "neither TENEX_EDGE_HOME nor HOME is set: refusing to relocate keystore/config/state.db \
     under ./.tenex-edge (would mint new agent identities and empty the trust whitelist)";

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
    /// the default `tenex-edge harness statusline` command. Use tmux format variables
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeHomeSelection {
    pub edge_home: PathBuf,
    pub default_edge_home: Option<PathBuf>,
    pub tenex_edge_home_set: bool,
    pub edge_home_is_default: bool,
}

/// tenex-edge's own writable root. Override with `$TENEX_EDGE_HOME` (tests use
/// this for isolation). Default: `~/.tenex-edge`.
pub fn edge_home() -> PathBuf {
    edge_home_selection().edge_home
}

pub fn edge_home_selection() -> EdgeHomeSelection {
    select_edge_home(
        std::env::var_os("TENEX_EDGE_HOME"),
        std::env::var_os("HOME"),
    )
    .unwrap_or_else(|message| panic!("{message}"))
}

pub fn isolated_home_acknowledged() -> bool {
    matches!(
        std::env::var(ISOLATED_HOME_ACK_ENV)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

pub fn ensure_dir(p: &Path) -> Result<()> {
    std::fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
    Ok(())
}

fn select_edge_home(
    tenex_edge_home: Option<OsString>,
    home: Option<OsString>,
) -> std::result::Result<EdgeHomeSelection, &'static str> {
    let default_edge_home = home
        .filter(|h| !h.as_os_str().is_empty())
        .map(PathBuf::from)
        .map(|h| h.join(".tenex-edge"));

    if let Some(edge_home) = tenex_edge_home {
        let edge_home = PathBuf::from(edge_home);
        let edge_home_is_default = default_edge_home
            .as_ref()
            .map(|default| default == &edge_home)
            .unwrap_or(false);
        return Ok(EdgeHomeSelection {
            edge_home,
            default_edge_home,
            tenex_edge_home_set: true,
            edge_home_is_default,
        });
    }

    let Some(edge_home) = default_edge_home.clone() else {
        return Err(MISSING_HOME_MESSAGE);
    };
    Ok(EdgeHomeSelection {
        edge_home,
        default_edge_home,
        tenex_edge_home_set: false,
        edge_home_is_default: true,
    })
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
mod tests;
