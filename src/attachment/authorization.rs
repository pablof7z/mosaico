//! Exact-key BUD-11 upload authorization. No relay work is created.

use anyhow::{Context, Result};
use base64::engine::{general_purpose::URL_SAFE_NO_PAD, Engine as _};
use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;

use crate::asset_hash::Sha256Hash;

const AUTH_LIFETIME_SECS: u64 = 5 * 60;

pub(super) fn sign_upload(keys: &Keys, hash: &Sha256Hash) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expiration = now + AUTH_LIFETIME_SECS;
    let tags = [
        Tag::parse(["t", "upload"]).map_err(|error| anyhow::anyhow!("BUD-11 t tag: {error}"))?,
        Tag::parse(["x", &hash.to_hex()])
            .map_err(|error| anyhow::anyhow!("BUD-11 x tag: {error}"))?,
        Tag::parse(["expiration", &expiration.to_string()])
            .map_err(|error| anyhow::anyhow!("BUD-11 expiration tag: {error}"))?,
    ];
    let event = EventBuilder::new(Kind::Custom(24_242), "Upload Blob")
        .tags(tags)
        .finalize(keys)
        .context("signing BUD-11 upload authorization")?;
    Ok(URL_SAFE_NO_PAD.encode(event.as_json().as_bytes()))
}

#[cfg(test)]
mod tests {
    use nostr::key::Keys;

    use super::*;

    #[test]
    fn bud11_signing_uses_the_exact_key_without_relay_work() {
        let keys = Keys::generate();
        let authorization = sign_upload(&keys, &Sha256Hash::of(b"blob")).unwrap();
        let bytes = URL_SAFE_NO_PAD.decode(authorization).unwrap();
        let event = nostr::event::Event::from_json(bytes).unwrap();

        assert_eq!(event.pubkey, keys.public_key());
        event.verify().unwrap();
    }
}
