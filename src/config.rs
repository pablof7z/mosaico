//! Device-level config + mosaico's own writable home.
//!
//! Mosaico reads `config.json` from the selected instance root (for `whitelistedPubkeys`,
//! explicit `relays`, `mcpRedirectOrigins`, and `backendName` as the host
//! label) and keeps all of its writable state under that root. Unset `MOSAICO`
//! uses `~/.mosaico`; named instances use `~/.mosaico-instances/<name>`.
//!
//! The daemon maintains a watched in-memory `Config` snapshot. A valid selected
//! document replacement applies immediately; relay routing or backend-identity
//! changes rebuild the relay-facing runtime without a daemon restart. Malformed
//! or transient edits retain the last good snapshot. Trust decisions that must
//! honour a withdrawal without a reload still read the document per request —
//! see [`mcp_trust`].

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

mod attachment_directory;
mod behavior;
mod document;
mod home;
mod management_key;
pub(crate) mod mcp_trust;
pub use behavior::{BoundaryAction, CrossProjectBoundary};
pub use harness_detection::detect as detect_available_harnesses;
#[path = "config/harness_detection.rs"]
mod harness_detection;
pub use home::{
    config_path, isolated_home_acknowledged, mosaico_home, mosaico_home_selection,
    selected_instance_env, validate_process_selection, MosaicoHomeSelection, INSTANCE_ENV,
    ISOLATED_HOME_ACK_ENV,
};
pub(crate) use management_key::{ensure_mosaico_private_key, generate_mosaico_private_key};

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
    /// admin in every channel group. Never used for group management,
    /// session-key derivation, or backend identity.
    pub user_nsec: Option<String>,
    /// This backend/daemon's own Nostr secret key (bech32 nsec or hex). The
    /// sole signer for NIP-29 group management, session-key derivation, and
    /// backend identity. Its pubkey is added as an admin to every group we
    /// create and is the address the orchestration listener matches `add`
    /// tags against.
    pub mosaico_private_key: Option<String>,
    /// Whether human-initiated sessions mint their own per-session NIP-29
    /// subgroup. Default `false`: such sessions
    /// land in the bare root channel, and a direct launch without `--channel`
    /// uses that root instead of minting a room.
    /// When `true`, per-session rooms are enabled (mint a per-session room).
    pub per_session_rooms: bool,
    /// Cooperative guardrails for explicit structured file-tool paths.
    pub cross_project_boundary: CrossProjectBoundary,
    /// Shared directory where received chat attachments are materialized.
    pub attachment_receive_directory: PathBuf,
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

/// Mirror of the relevant fields in the selected `config.json`. Unknown fields are
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
    #[serde(default)]
    agents: behavior::RawAgents,
    #[serde(default, rename = "attachmentReceiveDirectory")]
    attachment_receive_directory: Option<PathBuf>,
}

impl Config {
    /// Parse from JSON, resolving local storage against the selected Mosaico home.
    pub fn from_json_str(s: &str, fallback_host: &str) -> Result<Self> {
        Self::from_json_str_at(s, fallback_host, &mosaico_home())
    }

    fn from_json_str_at(s: &str, fallback_host: &str, home: &Path) -> Result<Self> {
        let raw: RawConfig = serde_json::from_str(s).context("parsing mosaico config json")?;
        let host = raw
            .backend_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| fallback_host.to_string());
        let indexer_relay = raw
            .indexer_relay
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_INDEXER_RELAY.to_string());
        let attachment_receive_directory =
            attachment_directory::resolve(raw.attachment_receive_directory, home)?;
        Ok(Config {
            whitelisted_pubkeys: raw.whitelisted_pubkeys,
            relays: raw.relays,
            indexer_relay,
            host,
            user_nsec: raw.user_nsec,
            mosaico_private_key: raw.mosaico_private_key,
            per_session_rooms: raw.per_session_rooms,
            cross_project_boundary: raw.agents.behavior.cross_project_boundary,
            attachment_receive_directory,
        })
    }

    /// Load from the selected instance config (or the low-level
    /// `$MOSAICO_CONFIG` override when no named selector is active).
    pub fn load() -> Result<Self> {
        let path = config_path();
        let s = std::fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "{} does not exist yet — run `mosaico setup` to create it",
                    path.display()
                )
            } else {
                anyhow::Error::new(e).context(format!("reading {}", path.display()))
            }
        })?;
        let config = Self::from_json_str(&s, &hostname())?;
        require_configured_relay(config)
    }
}

pub(crate) fn ensure_attachment_receive_directory() -> Result<PathBuf> {
    attachment_directory::ensure()
}

fn require_configured_relay(config: Config) -> Result<Config> {
    if config.relays.is_empty() {
        anyhow::bail!(
            "config has no fabric relay; run `mosaico setup --relay <ws-url>` with an externally operated NIP-29 relay"
        );
    }
    Ok(config)
}

pub fn ensure_dir(p: &Path) -> Result<()> {
    std::fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
    Ok(())
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
