//! Persisted-salt session key derivation and the read-side session identity.

use hmac::{Hmac, Mac};
use nostr::*;
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

/// Generate the random, non-secret salt persisted with a session pubkey. The
/// management key remains the only secret at rest; this salt only makes each
/// derived signer independently reconstructable without another session id.
pub fn new_session_signer_salt() -> String {
    Keys::generate().public_key().to_hex()
}

/// Reconstruct a session's signer from the per-machine management secret and
/// its persisted random salt. The resulting pubkey is the session identity;
/// neither a daemon row id nor a harness-native locator participates.
pub fn derive_session_keys(mgmt_secret: &SecretKey, signer_salt: &str) -> anyhow::Result<Keys> {
    const SALT: &[u8] = b"mosaico/session-pubkey/v3";
    let signer_salt = PublicKey::from_hex(signer_salt)
        .map_err(|error| anyhow::anyhow!("invalid session signer salt: {error}"))?;
    let signer_salt = signer_salt.as_bytes();
    let ikm = mgmt_secret.as_secret_bytes();
    let mut info = Vec::with_capacity(signer_salt.len() + 2);
    info.extend_from_slice(signer_salt);
    info.push(0x00);
    info.push(0x00);

    Ok(derive_keys_with_counter(
        ikm,
        SALT,
        info,
        "derive_session_keys",
    ))
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

/// The read-side identity of one running session. The pubkey is the identity;
/// the full handle is its one outward alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub pubkey: String,
    pub slug: String,
    pub handle: String,
    pub durable_agent: bool,
}

impl SessionIdentity {
    pub fn new(pubkey: String, slug: String, handle: String, durable_agent: bool) -> Self {
        Self {
            pubkey,
            slug,
            handle,
            durable_agent,
        }
    }

    pub fn durable_agent(pubkey: String, slug: String) -> Self {
        let handle = slug.clone();
        Self {
            pubkey,
            slug,
            handle,
            durable_agent: true,
        }
    }

    /// Public display uses the exact leased handle, never a runtime locator.
    pub fn display_slug(&self) -> String {
        self.handle.clone()
    }

    /// The wire reference: the session's own pubkey named by its public handle.
    pub fn agent_ref(&self) -> crate::domain::AgentRef {
        crate::domain::AgentRef::new(self.pubkey.clone(), self.display_slug())
    }
}
