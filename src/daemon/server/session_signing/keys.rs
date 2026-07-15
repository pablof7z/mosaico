//! Pubkey-keyed signer reconstruction.

use super::*;

impl DaemonState {
    /// Resolve the signer identified by `pubkey`. Durable agents use their
    /// configured key; ordinary sessions reconstruct from pubkey-bound salt.
    pub(in crate::daemon) fn session_signing_keys(&self, pubkey: &str) -> Result<Keys> {
        if let Some(signer_salt) = self.with_store(|store| store.session_signer_salt(pubkey))? {
            let mgmt = self.management_keys()?;
            let keys = crate::identity::derive_session_keys(mgmt.secret_key(), &signer_salt)?;
            if keys.public_key().to_hex() != pubkey {
                anyhow::bail!("stored signer salt does not reproduce session pubkey");
            }
            return Ok(keys);
        }

        let session = self
            .with_store(|store| store.get_session(pubkey))?
            .with_context(|| format!("pubkey {pubkey:?} has no local runtime projection"))?;
        let agent = crate::identity::load(&crate::config::mosaico_home(), &session.agent_slug)?;
        if agent.per_session_key || agent.pubkey_hex() != pubkey {
            anyhow::bail!(
                "durable signer configuration changed for agent {:?}",
                session.agent_slug
            );
        }
        Ok(agent.keys)
    }
}
