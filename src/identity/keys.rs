//! Deterministic per-session key derivation and the read-side session identity.

use hmac::{Hmac, Mac};
use nostr_sdk::prelude::*;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// HKDF-SHA256: Extract then Expand to produce exactly 32 bytes of keying
/// material. We only ever need one output block (L = 32 = HashLen).
fn hkdf_sha256_32(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(salt).expect("HMAC accepts any key length");
    mac.update(ikm);
    let prk: [u8; 32] = mac.finalize().into_bytes().into();

    let mut mac = HmacSha256::new_from_slice(&prk).expect("PRK is always 32 bytes");
    mac.update(info);
    mac.update(&[0x01u8]);
    mac.finalize().into_bytes().into()
}

/// Deterministically derive a session's OWN keypair from the per-machine
/// management secret and the canonical session id. Every session mints its own
/// keypair; the management key (`tenexPrivateKey`) is the only stored secret.
///
/// Determinism: a resumed session (same `session_id`) re-derives the identical
/// pubkey, so a p-tagged mention can route back to it. Cross-machine divergence:
/// the management secret is per-machine, so the same `session_id` on two machines
/// yields two different keypairs — a session identity is `(session, machine)`.
pub fn derive_session_keys_v2(mgmt_secret: &SecretKey, session_id: &str) -> Keys {
    const SALT: &[u8] = b"tenex-edge/session-pubkey/v2";
    let ikm = mgmt_secret.as_secret_bytes();
    let mut info = Vec::with_capacity(session_id.len() + 2);
    info.extend_from_slice(session_id.as_bytes());
    info.push(0x00);
    info.push(0x00);

    derive_keys_with_counter(ikm, SALT, info, "derive_session_keys_v2")
}

fn derive_keys_with_counter(ikm: &[u8], salt: &[u8], mut info: Vec<u8>, label: &str) -> Keys {
    let counter_idx = info.len() - 1;
    loop {
        let okm = hkdf_sha256_32(ikm, salt, &info);
        match SecretKey::from_slice(&okm) {
            Ok(sk) => return Keys::new(sk),
            Err(_) => {
                let counter = info[counter_idx];
                assert!(
                    counter < 255,
                    "{label}: exhausted rejection counter (astronomically improbable)"
                );
                info[counter_idx] = counter + 1;
            }
        }
    }
}

/// The read-side identity of one running session: its per-session pubkey, the
/// underlying agent slug (for the roster / local keystore), the canonical
/// session id, and the legacy memorable codename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub pubkey: String,
    pub slug: String,
    pub session_id: String,
    pub codename: String,
}

impl SessionIdentity {
    pub fn new(pubkey: String, slug: String, session_id: String, codename: String) -> Self {
        Self {
            pubkey,
            slug,
            session_id,
            codename,
        }
    }

    /// Projection when no bound `identities` row exists yet: the codename is the
    /// deterministic `friendly_short_code` of the session id, the same value the
    /// mint path would have persisted.
    pub fn fallback(session_id: &str, slug: String, pubkey: String) -> Self {
        Self {
            pubkey,
            slug,
            session_id: session_id.to_string(),
            codename: crate::util::friendly_short_code(session_id),
        }
    }

    /// The per-session display name: `agentSlug/session_id`.
    pub fn display_slug(&self) -> String {
        crate::idref::session_handle(&self.slug, &self.session_id)
    }

    /// The wire reference: the session's own pubkey named by its public handle.
    pub fn agent_ref(&self) -> crate::domain::AgentRef {
        crate::domain::AgentRef::new(self.pubkey.clone(), self.display_slug())
    }
}
