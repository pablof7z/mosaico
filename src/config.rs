//! Device-level config + mosaico's own writable home.
//!
//! mosaico *reads* `~/.mosaico/config.json` (for `whitelistedPubkeys`,
//! optional `relays`, and `backendName` as the host label) and keeps all of its
//! own writable state under `~/.mosaico`.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod management_key;
pub use harness_detection::detect as detect_available_harnesses;
#[path = "config/harness_detection.rs"]
mod harness_detection;
pub(crate) use management_key::{ensure_mosaico_private_key, generate_mosaico_private_key};

pub const DEFAULT_RELAY: &str = "wss://nip29.f7z.io";
pub const DEFAULT_INDEXER_RELAY: &str = "wss://purplepag.es";
pub const ISOLATED_HOME_ACK_ENV: &str = "MOSAICO_ISOLATED_HOME_OK";
const MISSING_HOME_MESSAGE: &str =
    "neither MOSAICO_HOME nor HOME is set: refusing to relocate keystore/config/state.db \
     under ./.mosaico (would mint new agent identities and empty the trust whitelist)";

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
    /// admin in every channel group. Never used for group management,
    /// session-key derivation, or backend identity.
    pub user_nsec: Option<String>,
    /// This backend/daemon's own Nostr secret key (bech32 nsec or hex). The
    /// sole signer for NIP-29 group management, session-key derivation, and
    /// backend identity. Its pubkey is added as an admin to every group we
    /// create and is the address the orchestration listener matches `add`
    /// tags against.
    pub mosaico_private_key: Option<String>,
    /// Whether human-initiated sessions (no `MOSAICO_CHANNEL` override) mint
    /// their own per-session NIP-29 subgroup. Default `false`: such sessions
    /// land in the bare root channel, and `mosaico agents` (without
    /// `--channel`) opens the interactive channel picker instead of minting.
    /// When `true`, per-session rooms are enabled (mint a per-session room).
    pub per_session_rooms: bool,
}

impl Config {
    /// Key used as the HKDF IKM for per-session key derivation. The backend's
    /// own key (`mosaicoPrivateKey`) — never the operator's `userNsec`.
    pub fn session_ikm_nsec(&self) -> Option<&String> {
        self.mosaico_private_key.as_ref()
    }

    /// Signer for NIP-29 group-management events (create/lock/put-user/
    /// put-admin/remove-user/edit-metadata). Always the backend's own
    /// `mosaicoPrivateKey` — the operator's `userNsec` is no longer used for
    /// group management. The operator's pubkey is instead *granted* the admin
    /// role by this signer (see `Nip29Provider::open_channel`).
    pub fn management_nsec(&self) -> Option<&String> {
        self.mosaico_private_key.as_ref()
    }

    /// This backend's own identity key. Always `mosaicoPrivateKey`; there is no
    /// fallback to `userNsec` — the operator key is a human identity, not a
    /// backend identity.
    pub fn backend_nsec(&self) -> Option<&String> {
        self.mosaico_private_key.as_ref()
    }

    /// The human operator's Nostr secret key. Used by
    /// `try_grant_mgmt_admin_via_user_nsec` to sign the one-time grant of the
    /// admin role to the backend's management key on a newly-opened group. The
    /// operator's pubkey is NOT derived from this field for that grant — it
    /// lives in `whitelisted_pubkeys` instead. Never used for session-key
    /// derivation or backend identity.
    pub fn user_nsec(&self) -> Option<&String> {
        self.user_nsec.as_ref()
    }
}

/// Mirror of the relevant fields in `~/.mosaico/config.json`. Unknown fields are
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
    #[serde(default, rename = "mosaicoPrivateKey")]
    mosaico_private_key: Option<String>,
    /// Opt-in: mint a per-session subgroup for human-initiated sessions.
    /// Defaults to `false` (use the root channel; `launch` opens the picker).
    #[serde(default, rename = "perSessionRooms")]
    per_session_rooms: bool,
}

impl Config {
    /// Parse from a JSON string. Pure — the unit-testable core of `load`.
    pub fn from_json_str(s: &str, fallback_host: &str) -> Result<Self> {
        let raw: RawConfig = serde_json::from_str(s).context("parsing mosaico config json")?;
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
            mosaico_private_key: raw.mosaico_private_key,
            per_session_rooms: raw.per_session_rooms,
        })
    }

    /// Load from `~/.mosaico/config.json` (or `$MOSAICO_CONFIG` override).
    pub fn load() -> Result<Self> {
        let path = config_path();
        let s = std::fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "{} does not exist yet — run `mosaico install` to set it up",
                    path.display()
                )
            } else {
                anyhow::Error::new(e).context(format!("reading {}", path.display()))
            }
        })?;
        Self::from_json_str(&s, &hostname())
    }
}

pub fn config_path() -> PathBuf {
    select_config_path(std::env::var_os("MOSAICO_CONFIG"), mosaico_home())
}

fn select_config_path(mosaico_config: Option<OsString>, mosaico_home: PathBuf) -> PathBuf {
    match mosaico_config {
        Some(p) => PathBuf::from(p),
        None => mosaico_home.join("config.json"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MosaicoHomeSelection {
    pub mosaico_home: PathBuf,
    pub default_mosaico_home: Option<PathBuf>,
    pub mosaico_home_set: bool,
    pub mosaico_home_is_default: bool,
}

/// mosaico's own writable root. Override with `$MOSAICO_HOME` (tests use
/// this for isolation). Default: `~/.mosaico`.
pub fn mosaico_home() -> PathBuf {
    mosaico_home_selection().mosaico_home
}

pub fn mosaico_home_selection() -> MosaicoHomeSelection {
    select_mosaico_home(std::env::var_os("MOSAICO_HOME"), std::env::var_os("HOME"))
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

fn select_mosaico_home(
    mosaico_home: Option<OsString>,
    home: Option<OsString>,
) -> std::result::Result<MosaicoHomeSelection, &'static str> {
    let default_mosaico_home = home
        .filter(|h| !h.as_os_str().is_empty())
        .map(PathBuf::from)
        .map(|h| h.join(".mosaico"));

    if let Some(mosaico_home) = mosaico_home {
        let mosaico_home = PathBuf::from(mosaico_home);
        let mosaico_home_is_default = default_mosaico_home
            .as_ref()
            .map(|default| default == &mosaico_home)
            .unwrap_or(false);
        return Ok(MosaicoHomeSelection {
            mosaico_home,
            default_mosaico_home,
            mosaico_home_set: true,
            mosaico_home_is_default,
        });
    }

    let Some(mosaico_home) = default_mosaico_home.clone() else {
        return Err(MISSING_HOME_MESSAGE);
    };
    Ok(MosaicoHomeSelection {
        mosaico_home,
        default_mosaico_home,
        mosaico_home_set: false,
        mosaico_home_is_default: true,
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
