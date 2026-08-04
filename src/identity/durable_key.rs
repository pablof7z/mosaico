use super::{atomic_write, key_path, read_stored_key_unvalidated, validate_slug};
use anyhow::{bail, Context, Result};
use nostr::Keys;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DurableKeyStatus {
    Absent,
    Ready,
    Missing,
}

pub(crate) fn durable_key_status(mosaico_home: &Path, slug: &str) -> Result<DurableKeyStatus> {
    validate_slug(slug)?;
    let path = key_path(mosaico_home, slug);
    if !path.exists() {
        return Ok(DurableKeyStatus::Absent);
    }
    let stored = read_stored_key_unvalidated(&path)?;
    if stored.per_session_key {
        stored
            .identity_keys()
            .with_context(|| format!("validating agent record {}", path.display()))?;
        return Ok(DurableKeyStatus::Ready);
    }

    match (stored.secret_key.as_deref(), stored.public_key.as_deref()) {
        (None, _) => Ok(DurableKeyStatus::Missing),
        (Some(secret), None) => {
            Keys::parse(secret)
                .with_context(|| format!("parsing durable secret key for {:?}", stored.slug))?;
            Ok(DurableKeyStatus::Missing)
        }
        (Some(_), Some(_)) => {
            stored
                .identity_keys()
                .with_context(|| format!("validating agent record {}", path.display()))?;
            Ok(DurableKeyStatus::Ready)
        }
    }
}

/// Complete a durable agent's missing key material without changing any other
/// launch configuration. If a valid secret already exists, derive only its
/// missing public key; otherwise create one fresh matching keypair.
pub(crate) fn create_missing_durable_key(mosaico_home: &Path, slug: &str) -> Result<bool> {
    validate_slug(slug)?;
    let path = key_path(mosaico_home, slug);
    if !path.exists() {
        bail!("no configured agent named {slug:?}");
    }
    let mut stored = read_stored_key_unvalidated(&path)?;
    if stored.per_session_key {
        bail!("agent {slug:?} uses per-session keys");
    }

    match (stored.secret_key.as_deref(), stored.public_key.as_deref()) {
        (Some(_), Some(_)) => {
            stored
                .identity_keys()
                .with_context(|| format!("validating agent record {}", path.display()))?;
            return Ok(false);
        }
        (Some(secret), None) => {
            let keys = Keys::parse(secret)
                .with_context(|| format!("parsing durable secret key for {:?}", stored.slug))?;
            stored.public_key = Some(keys.public_key().to_hex());
        }
        (None, _) => {
            let keys = Keys::generate();
            stored.secret_key = Some(keys.secret_key().to_secret_hex());
            stored.public_key = Some(keys.public_key().to_hex());
        }
    }

    stored
        .identity_keys()
        .with_context(|| format!("validating repaired agent record {}", path.display()))?;
    atomic_write(&path, &serde_json::to_string_pretty(&stored)?)?;
    Ok(true)
}
