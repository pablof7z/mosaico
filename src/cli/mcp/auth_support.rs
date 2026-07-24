use anyhow::{Context, Result};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use nostr::PublicKey;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use url::Url;

type HmacSha256 = Hmac<Sha256>;

pub(super) fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

pub(super) fn scope_allowed(granted: &str, required: &str) -> bool {
    granted.split_whitespace().any(|scope| scope == required)
}

pub(super) fn normalize_pubkey(value: &str) -> String {
    PublicKey::parse(value)
        .map(|pk| pk.to_hex())
        .unwrap_or_else(|_| value.to_ascii_lowercase())
}

pub(super) fn random_token(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    File::open("/dev/urandom")
        .context("opening /dev/urandom")?
        .read_exact(&mut buf)
        .context("reading random bytes")?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

pub(super) fn sign(secret: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(payload);
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

pub(super) fn stable_hash(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    URL_SAFE_NO_PAD.encode(&Sha256::digest(bytes)[..12])
}

pub(super) fn redirect_with_code(redirect_uri: &str, code: &str, state: Option<&str>) -> Response {
    let mut url = match Url::parse(redirect_uri) {
        Ok(url) => url,
        Err(err) => return oauth_error(StatusCode::BAD_REQUEST, err.to_string()),
    };
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("code", code);
        if let Some(state) = state {
            query.append_pair("state", state);
        }
    }
    Redirect::to(url.as_str()).into_response()
}

pub(super) fn oauth_json_error(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

pub(super) fn oauth_error(status: StatusCode, message: String) -> Response {
    (status, message).into_response()
}
