//! Restore NIP-42 capabilities before durable relay work resumes.

use std::sync::Arc;

use anyhow::{Context, Result};
use nostr::Keys;

use crate::{
    config::{self, Config},
    daemon::server::DaemonState,
};

pub(super) fn load_backend() -> Result<(Config, Keys)> {
    let mut cfg = Config::load().context("loading config")?;
    let backend_nsec = match cfg.backend_nsec().filter(|key| !key.trim().is_empty()) {
        Some(key) => key.clone(),
        None => {
            let key = config::ensure_mosaico_private_key()
                .context("ensuring stable backend identity for NIP-42")?;
            cfg = Config::load().context("reloading config with backend identity")?;
            key
        }
    };
    let keys =
        Keys::parse(&backend_nsec).context("mosaicoPrivateKey is not a valid Nostr secret key")?;
    Ok((cfg, keys))
}

pub(super) fn restore(state: &Arc<DaemonState>) -> Result<()> {
    if let Some(user_nsec) = state.cfg.user_nsec() {
        match Keys::parse(user_nsec) {
            Ok(keys) => register(state, &keys, "operator")?,
            Err(error) => tracing::warn!(
                error = %error,
                "operator key is invalid; its NIP-42 capability was not restored"
            ),
        }
    }

    let pubkeys = state.with_store(|store| store.list_local_session_pubkeys().unwrap_or_default());
    for pubkey in pubkeys {
        match state.session_signing_keys(&pubkey) {
            Ok(keys) => register(state, &keys, "session")?,
            Err(error) => tracing::warn!(
                pubkey,
                error = %error,
                "local signer could not be reconstructed for NIP-42 restore"
            ),
        }
    }
    Ok(())
}

fn register(state: &Arc<DaemonState>, keys: &Keys, identity_kind: &'static str) -> Result<()> {
    state.nmp.ensure_identity(keys).with_context(|| {
        format!(
            "registering {identity_kind} NIP-42 identity {}",
            keys.public_key()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restores_a_reconstructible_session_before_new_activity() {
        let state = DaemonState::new_for_test().await;
        let salt = crate::identity::new_session_signer_salt();
        let backend = state.management_keys().unwrap();
        let keys = crate::identity::derive_session_keys(backend.secret_key(), &salt).unwrap();
        let pubkey = keys.public_key();
        state.with_store(|store| store.bind_session_signer(&pubkey.to_hex(), &salt).unwrap());

        assert!(!state.nmp.identity_registered(pubkey));
        restore(&state).unwrap();
        assert!(state.nmp.identity_registered(pubkey));
    }
}
