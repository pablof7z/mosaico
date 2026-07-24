//! Identity-scoped NIP-42 policy registration for NMP relay sessions.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use nmp::{
    AccountRegistration, AuthPolicy, AuthPolicyOp, AuthPolicyRegistration, AuthPolicyRequest,
    RelayUrl,
};
use nostr::{Keys, PublicKey};

use super::NmpHost;

pub(super) struct IdentityRegistration {
    _auth_policy: AuthPolicyRegistration,
    _account: AccountRegistration,
}

#[derive(Clone)]
struct ConfiguredRelayAuthPolicy {
    expected_pubkey: PublicKey,
    allowed_relays: BTreeSet<RelayUrl>,
}

impl ConfiguredRelayAuthPolicy {
    fn allows(&self, expected_pubkey: PublicKey, relay: &RelayUrl) -> bool {
        expected_pubkey == self.expected_pubkey && self.allowed_relays.contains(relay)
    }
}

impl AuthPolicy for ConfiguredRelayAuthPolicy {
    fn evaluate(&self, request: AuthPolicyRequest) -> AuthPolicyOp {
        if self.allows(request.expected_pubkey(), request.relay()) {
            AuthPolicyOp::allow()
        } else {
            AuthPolicyOp::deny("relay or identity is outside Mosaico's configured AUTH scope")
        }
    }
}

impl NmpHost {
    /// Install the signer and exact-account NIP-42 policy as one retained
    /// capability pair. NMP freezes the requested identity into each relay
    /// session; this policy only approves configured app/indexer relays.
    pub(crate) fn ensure_identity(&self, keys: &Keys) -> Result<()> {
        let pubkey = keys.public_key();
        let mut identities = self
            .identities
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if identities.contains_key(&pubkey) {
            return Ok(());
        }

        let account = self
            .engine
            .add_account(&keys.secret_key().to_secret_hex())
            .with_context(|| format!("registering NMP account {pubkey}"))?;
        let policy = ConfiguredRelayAuthPolicy {
            expected_pubkey: pubkey,
            allowed_relays: self.profile_relays.clone(),
        };
        let auth_policy = match self.engine.add_auth_policy(pubkey, policy) {
            Ok(registration) => registration,
            Err(error) => {
                let cleanup = self.engine.remove_account(&account);
                return match cleanup {
                    Ok(_) => Err(error)
                        .with_context(|| format!("registering NIP-42 policy for {pubkey}")),
                    Err(cleanup_error) => Err(anyhow::anyhow!(
                        "registering NIP-42 policy for {pubkey}: {error}; account rollback failed: {cleanup_error}"
                    )),
                };
            }
        };
        identities.insert(
            pubkey,
            IdentityRegistration {
                _auth_policy: auth_policy,
                _account: account,
            },
        );
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn identity_registered(&self, pubkey: PublicKey) -> bool {
        self.identities
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains_key(&pubkey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_is_scoped_to_the_exact_identity_and_configured_relay() {
        let expected = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        let allowed = RelayUrl::parse("wss://relay.example.com").unwrap();
        let unknown = RelayUrl::parse("wss://unknown.example.com").unwrap();
        let policy = ConfiguredRelayAuthPolicy {
            expected_pubkey: expected,
            allowed_relays: BTreeSet::from([allowed.clone()]),
        };

        assert!(policy.allows(expected, &allowed));
        assert!(!policy.allows(other, &allowed));
        assert!(!policy.allows(expected, &unknown));
    }
}
