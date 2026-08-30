//! Dynamic Fava signer registration for Mosaico-managed identities.

use anyhow::{Context, Result};
use fava_signer_local::LocalSigner;
use nostr::key::Keys;
use std::sync::Arc;

use super::NmpHost;

impl NmpHost {
    /// Attach this exact local identity to Fava's signer session.
    ///
    /// Re-registering the same public key is harmless: the existing Fava
    /// attachment remains authoritative until a deliberate replacement.
    pub(crate) fn register_signer(&self, keys: &Keys) -> Result<()> {
        let public_key = keys.public_key();
        let signer = Arc::new(LocalSigner::new(keys.clone()));
        match self.fava.add_signer(signer) {
            Ok(()) => Ok(()),
            Err(fava::SessionError::DuplicateSigner(existing)) if existing == public_key => Ok(()),
            Err(error) => Err(error).with_context(|| format!("attaching signer {public_key}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dynamically_registered_signer_is_used_by_fava() {
        let backend = Keys::generate();
        let agent = Keys::generate();
        let host = NmpHost::open(&[], None, None, &backend).expect("open Fava host");
        host.register_signer(&agent).expect("attach agent signer");

        let unsigned = fava::EventBuilder::new(fava::Kind::from_u16(1))
            .content("unsigned from Mosaico")
            .by(agent.public_key())
            .build()
            .expect("build unsigned event");
        let signed = host.fava.sign(unsigned).await.expect("Fava signs event");

        assert_eq!(signed.pubkey, agent.public_key());
        assert!(signed.verify().is_ok(), "Fava-produced signature verifies");
        host.shutdown();
    }
}
