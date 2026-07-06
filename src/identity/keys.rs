//! Deterministic agent key derivation and instance identity.

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

/// Deterministically derive a per-session keypair. Same inputs produce the same
/// key, so a resumed harness session reproduces its pubkey.
pub fn derive_session_keys(
    tenex_secret: &SecretKey,
    project_slug: &str,
    agent_slug: &str,
    harness_kind: &str,
    anchor: &str,
) -> Keys {
    const SALT: &[u8] = b"tenex-edge/session-key/v1";
    let ikm = tenex_secret.as_secret_bytes();
    let mut info = Vec::with_capacity(
        project_slug.len() + 1 + agent_slug.len() + 1 + harness_kind.len() + 1 + anchor.len() + 2,
    );
    info.extend_from_slice(project_slug.as_bytes());
    info.push(0x00);
    info.extend_from_slice(agent_slug.as_bytes());
    info.push(0x00);
    info.extend_from_slice(harness_kind.as_bytes());
    info.push(0x00);
    info.extend_from_slice(anchor.as_bytes());
    info.push(0x00);
    info.push(0x00);

    derive_keys_with_counter(ikm, SALT, info, "derive_session_keys")
}

/// Display label for an agent's Nth concurrent identity.
pub fn agent_ordinal_label(agent_slug: &str, ordinal: u32) -> String {
    format!("{agent_slug}{ordinal}")
}

/// Deterministically derive the keypair for an agent's Nth concurrent identity.
/// Ordinals are stable across rooms and sessions for the same local derivation
/// root. Runtime identities start at ordinal 1; ordinal 0 is kept only for
/// legacy rows and is also derived, not the file-backed root key.
pub fn derive_agent_ordinal_keys(base: &Keys, ordinal: u32) -> Keys {
    const SALT: &[u8] = b"tenex-edge/agent-ordinal-key/v1";
    let ikm = base.secret_key().as_secret_bytes();
    let base_pub = base.public_key().to_hex();
    let mut info = Vec::with_capacity(base_pub.len() + 7);
    info.extend_from_slice(base_pub.as_bytes());
    info.push(0x00);
    info.extend_from_slice(&ordinal.to_be_bytes());
    info.push(0x00);
    info.push(0x00);

    derive_keys_with_counter(ikm, SALT, info, "derive_agent_ordinal_keys")
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

/// The single authoritative identity of one running agent instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstance {
    pub base_slug: String,
    pub base_pubkey: String,
    pub ordinal: u32,
    pub pubkey: String,
}

impl AgentInstance {
    pub fn base(base_slug: String, base_pubkey: String) -> Self {
        Self {
            base_slug,
            base_pubkey: base_pubkey.clone(),
            ordinal: 1,
            pubkey: base_pubkey,
        }
    }

    pub fn from_parts(
        base_slug: String,
        base_pubkey: String,
        ordinal: u32,
        pubkey: String,
    ) -> Self {
        Self {
            base_slug,
            base_pubkey,
            ordinal,
            pubkey,
        }
    }

    pub fn display_slug(&self) -> String {
        agent_ordinal_label(&self.base_slug, self.ordinal)
    }

    pub fn agent_ref(&self) -> crate::domain::AgentRef {
        crate::domain::AgentRef::new(self.pubkey.clone(), self.display_slug())
    }

    pub fn signing_keys(&self, base_keys: &Keys) -> Keys {
        if self.pubkey == base_keys.public_key().to_hex() {
            return base_keys.clone();
        }
        derive_agent_ordinal_keys(base_keys, self.ordinal)
    }
}
