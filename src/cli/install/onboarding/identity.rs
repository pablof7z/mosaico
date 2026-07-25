//! The operator identity and device name onboarding starts from.
//!
//! Both are derived once, before any decision the operator makes, so the state
//! machine can stay free of key generation and hostname handling.

use anyhow::{Context, Result};
use nostr::{Keys, ToBech32};

pub(super) const DEVICE_NAME_CAP: usize = 18;

/// The generated operator identity, shown once and persisted to `config.json`.
pub(super) struct Identity {
    pub nsec: String,
    pub npub: String,
    pub pubkey_hex: String,
}

pub(super) fn generate_identity() -> Result<Identity> {
    let keys = Keys::generate();
    Ok(Identity {
        nsec: keys
            .secret_key()
            .to_bech32()
            .context("encoding operator nsec")?,
        npub: keys
            .public_key()
            .to_bech32()
            .context("encoding operator npub")?,
        pubkey_hex: keys.public_key().to_hex(),
    })
}

/// Default device name: the slugified hostname, capped and hyphen-trimmed.
pub(super) fn default_device_name() -> String {
    let slug = crate::slug::slugify_host(&crate::config::hostname());
    let capped: String = slug.chars().take(DEVICE_NAME_CAP).collect();
    let trimmed = capped.trim_end_matches('-');
    if trimmed.is_empty() {
        "mosaico".to_string()
    } else {
        trimmed.to_string()
    }
}
