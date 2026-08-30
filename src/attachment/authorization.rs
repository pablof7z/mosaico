//! BUD-11 upload authorization through Fava's registered signer.

use std::sync::Arc;

use anyhow::{Context, Result};
use base64::engine::{general_purpose::URL_SAFE_NO_PAD, Engine as _};
use nostr::event::Tag;
use nostr::key::PublicKey;

use crate::asset_hash::Sha256Hash;
use crate::fava_host::FavaHost;

const AUTH_LIFETIME_SECS: u64 = 5 * 60;

pub(super) async fn sign_upload(
    fava: &Arc<FavaHost>,
    author: PublicKey,
    hash: &Sha256Hash,
) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expiration = now + AUTH_LIFETIME_SECS;
    let t_tag = Tag::parse(["t", "upload"]).map_err(|e| anyhow::anyhow!("BUD-11 t tag: {e}"))?;
    let x_tag =
        Tag::parse(["x", &hash.to_hex()]).map_err(|e| anyhow::anyhow!("BUD-11 x tag: {e}"))?;
    let exp_tag = Tag::parse(["expiration", &expiration.to_string()])
        .map_err(|e| anyhow::anyhow!("BUD-11 expiration tag: {e}"))?;
    let unsigned = fava::EventBuilder::new(fava::Kind::from_u16(24242))
        .content("Upload Blob")
        .tags([t_tag, x_tag, exp_tag])
        .by(author)
        .build()
        .map_err(anyhow::Error::from)?;
    let signed = fava
        .fava
        .sign(unsigned)
        .await
        .map_err(anyhow::Error::from)
        .context("Fava signing BUD-11 upload authorization")?;
    let json = signed.as_json();
    Ok(URL_SAFE_NO_PAD.encode(json.as_bytes()))
}
