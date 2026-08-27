//! NIP-42 auth is handled automatically by fava's LocalSigner.
//!
//! With NMP, each pubkey needed explicit signer and AUTH-policy registration.
//! With fava, signers are registered at build time and the transport layer
//! handles NIP-42 challenges for all registered pubkeys automatically.
//!
//! This module is retained as a thin compatibility shim for call sites that
//! called `ensure_identity` to register a dynamic keypair. With fava there is
//! nothing to do — the backend signer is registered once at `NmpHost::open`.

use anyhow::Result;
use nostr::key::Keys;

use super::NmpHost;

impl NmpHost {
    /// Compatibility shim. With fava, identity registration happens at build
    /// time via `Fava::builder().signer(...)`. This is a no-op.
    pub(crate) fn ensure_identity(&self, _keys: &Keys) -> Result<()> {
        Ok(())
    }

    /// Test-facing: with fava the backend signer is always registered.
    #[cfg(test)]
    pub(crate) fn has_registered_identity(&self, _pubkey: &nostr::key::PublicKey) -> bool {
        true
    }
}
