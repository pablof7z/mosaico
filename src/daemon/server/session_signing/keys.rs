//! Pubkey-keyed signer reconstruction.

use super::*;

impl DaemonState {
    /// Resolve the signer identified by `pubkey`. Durable agents use their
    /// configured key; ordinary sessions reconstruct from pubkey-bound salt.
    pub(in crate::daemon) fn session_signing_keys(&self, pubkey: &str) -> Result<Keys> {
        let durable = self.with_store(|store| store.is_durable_agent_pubkey(pubkey))?;
        let keys = if durable {
            let identity = self
                .with_store(|store| store.get_identity(pubkey))?
                .with_context(|| format!("pubkey {pubkey:?} has no signing identity"))?;
            let agent = crate::identity::load_or_create(
                &crate::config::edge_home(),
                &identity.agent_slug,
                crate::util::now_secs(),
            )?;
            if agent.per_session_key || agent.pubkey_hex() != pubkey {
                anyhow::bail!(
                    "durable signer configuration changed for agent {:?}",
                    identity.agent_slug
                );
            }
            agent.keys
        } else {
            let mgmt = self.management_keys()?;
            let signer_salt = self
                .with_store(|store| store.session_signer_salt(pubkey))?
                .with_context(|| format!("pubkey {pubkey:?} has no signer material"))?;
            let keys = crate::identity::derive_session_keys(mgmt.secret_key(), &signer_salt)?;
            if keys.public_key().to_hex() != pubkey {
                anyhow::bail!("stored signer salt does not reproduce session pubkey");
            }
            keys
        };
        Ok(keys)
    }
}
