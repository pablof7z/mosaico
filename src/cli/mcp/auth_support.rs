use anyhow::{Context, Result};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use nostr_sdk::prelude::PublicKey;
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

pub(super) fn login_html(
    fields: &[(String, String)],
    error: Option<&str>,
    authorize_url: &str,
) -> String {
    let error = error
        .map(|e| format!("<p class=\"error\">{}</p>", html(e)))
        .unwrap_or_default();
    let inputs = fields
        .iter()
        .map(|(name, value)| {
            format!(
                r#"<input type="hidden" name="{}" value="{}">"#,
                html(name),
                html(value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script = r#"<script>
const form = document.getElementById("login-form");
const button = document.getElementById("nip07-button");
const status = document.getElementById("login-status");
const nsecInput = document.getElementById("nsec-input");

button.addEventListener("click", async () => {
  try {
    if (!window.nostr || !window.nostr.getPublicKey || !window.nostr.signEvent) {
      throw new Error("No NIP-07 signer was found in this browser.");
    }
    button.disabled = true;
    status.textContent = "Waiting for signer approval...";
    const pubkey = await window.nostr.getPublicKey();
    const event = await window.nostr.signEvent({
      kind: 27235,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["u", form.dataset.authorizeUrl],
        ["method", "POST"],
        ["challenge", form.elements.login_challenge.value],
        ["client", "tenex-edge-mcp"]
      ],
      content: "tenex-edge OAuth login"
    });
    form.elements.nip07_pubkey.value = pubkey;
    form.elements.nip07_event.value = JSON.stringify(event);
    nsecInput.required = false;
    form.submit();
  } catch (err) {
    status.textContent = err && err.message ? err.message : String(err);
    button.disabled = false;
  }
});
</script>"#;
    format!(
        r#"<!doctype html>
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>tenex-edge login</title>
<style>
body{{font-family:system-ui,sans-serif;margin:2rem;max-width:38rem}}
label,input,button{{display:block;width:100%;box-sizing:border-box}}
input{{padding:.7rem;margin:.4rem 0 1rem}}button{{padding:.8rem;margin:.7rem 0}}
.error{{color:#a40000}}.hint,.status{{color:#555}}.fallback{{margin-top:1.5rem;border-top:1px solid #ddd;padding-top:1rem}}
</style>
<h1>tenex-edge login</h1>
<p class="hint">Pair with a Nostr signer whose public key is listed in whitelistedPubkeys.</p>
{error}
<form id="login-form" method="post" action="/oauth/authorize" data-authorize-url="{authorize_url}">
{inputs}
<input name="nip07_pubkey" type="hidden">
<input name="nip07_event" type="hidden">
<button id="nip07-button" type="button">Pair with NIP-07</button>
<p id="login-status" class="status"></p>
<div class="fallback">
<label>nsec<input id="nsec-input" name="nsec" type="password" autocomplete="off" required></label>
<button type="submit">Pair with nsec</button>
</div>
</form>
{script}"#,
        authorize_url = html(authorize_url),
        inputs = inputs,
        error = error,
        script = script,
    )
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
