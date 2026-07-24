use anyhow::{Context, Result};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use nostr::Keys;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::auth_login_page::login_html;
use super::auth_support::{
    bearer, normalize_pubkey, oauth_error, oauth_json_error, random_token, redirect_with_code,
    scope_allowed, sign, stable_hash,
};
use super::auth_types::{validate_token_request, AuthCode, LoginChallenge};
pub(super) use super::auth_types::{AuthorizeForm, AuthorizeParams, TokenForm};

const SCOPES: &[&str] = &["mosaico:read", "mosaico:write"];

#[derive(Clone)]
pub(super) struct AuthState {
    public_url: String,
    resource_url: String,
    secret: Vec<u8>,
    whitelisted: Arc<Vec<String>>,
    codes: Arc<Mutex<HashMap<String, AuthCode>>>,
    challenges: Arc<Mutex<HashMap<String, LoginChallenge>>>,
}

#[derive(Deserialize, Serialize)]
struct TokenClaims {
    iss: String,
    aud: String,
    sub: String,
    scope: String,
    iat: u64,
}

pub(super) struct Authenticated {
    pub(super) subject: String,
}

impl AuthState {
    pub(super) fn new(public_url: String, resource_path: &str) -> Result<Self> {
        let cfg = crate::config::Config::load()?;
        let secret = match cfg.management_nsec().cloned() {
            Some(secret) => secret,
            None => crate::config::ensure_mosaico_private_key()?,
        };
        let public_url = public_url.trim_end_matches('/').to_string();
        let resource_url = format!("{public_url}{resource_path}");
        Ok(Self {
            public_url,
            resource_url,
            secret: secret.into_bytes(),
            whitelisted: Arc::new(cfg.whitelisted_pubkeys),
            codes: Arc::new(Mutex::new(HashMap::new())),
            challenges: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(super) fn protected_resource(&self) -> Value {
        json!({
            "resource": self.resource_url,
            "authorization_servers": [self.public_url],
            "scopes_supported": SCOPES,
            "resource_documentation": "https://github.com/pablof7z/mosaico",
        })
    }

    pub(super) fn authorization_server(&self) -> Value {
        json!({
            "issuer": self.public_url,
            "authorization_endpoint": format!("{}/oauth/authorize", self.public_url),
            "token_endpoint": format!("{}/oauth/token", self.public_url),
            "registration_endpoint": format!("{}/oauth/register", self.public_url),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "client_id_metadata_document_supported": true,
            "scopes_supported": SCOPES,
        })
    }

    pub(super) async fn authorize_page(&self, params: AuthorizeParams) -> Response {
        if let Err(err) = params.validate(&self.resource_url) {
            return oauth_error(StatusCode::BAD_REQUEST, err.to_string());
        }
        self.login_page(&params, None).await
    }

    pub(super) async fn authorize_submit(&self, form: AuthorizeForm) -> Response {
        let params = form.params();
        if let Err(err) = params.validate(&self.resource_url) {
            return oauth_error(StatusCode::BAD_REQUEST, err.to_string());
        }
        let challenge = match self.consume_challenge(&form, &params).await {
            Ok(challenge) => challenge,
            Err(err) => return self.login_page(&params, Some(&err.to_string())).await,
        };
        let pubkey = match self.pubkey_for_login(&form, &challenge) {
            Ok(pubkey) => pubkey,
            Err(err) => return self.login_page(&params, Some(&err.to_string())).await,
        };
        let code = match random_token(32) {
            Ok(code) => code,
            Err(err) => return oauth_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        };
        let record = AuthCode {
            client_id: params.client_id.clone(),
            redirect_uri: params.redirect_uri.clone(),
            code_challenge: params.code_challenge.clone(),
            resource: params.resource_url(&self.resource_url),
            scope: params.scope.clone().unwrap_or_else(default_scope),
            pubkey,
            expires_at: crate::util::now_secs() + 300,
        };
        self.codes.lock().await.insert(code.clone(), record);
        redirect_with_code(&params.redirect_uri, &code, params.state.as_deref())
    }

    pub(super) async fn token(&self, form: TokenForm) -> Response {
        if form.grant_type != "authorization_code" {
            return oauth_json_error("unsupported_grant_type", "authorization_code required");
        }
        let Some(code) = self.codes.lock().await.remove(&form.code) else {
            return oauth_json_error("invalid_grant", "unknown authorization code");
        };
        if let Err(err) = validate_token_request(&form, &code, &self.resource_url) {
            return oauth_json_error("invalid_grant", &err.to_string());
        }
        match self.issue_token(&code) {
            Ok(token) => Json(json!({
                "access_token": token,
                "token_type": "Bearer",
                "scope": code.scope,
            }))
            .into_response(),
            Err(err) => oauth_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        }
    }

    pub(super) fn register(&self, body: Value) -> Response {
        let redirect_uris = body
            .get("redirect_uris")
            .cloned()
            .unwrap_or_else(|| json!([]));
        Json(json!({
            "client_id": format!("{}/oauth/client/{}", self.public_url, stable_hash(&body)),
            "redirect_uris": redirect_uris,
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }))
        .into_response()
    }

    pub(super) fn verify(
        &self,
        headers: &HeaderMap,
        scope: &str,
    ) -> Result<Authenticated, Box<Response>> {
        let token = bearer(headers).ok_or_else(|| Box::new(self.challenge()))?;
        let claims = self
            .verify_token(token)
            .map_err(|_| Box::new(self.challenge()))?;
        if !scope_allowed(&claims.scope, scope) {
            return Err(Box::new(self.challenge()));
        }
        Ok(Authenticated {
            subject: claims.sub,
        })
    }

    pub(super) fn redact_actor_key(&self, fields: &[&str]) -> String {
        let joined = fields.join("\u{1f}");
        format!("mcp1_{}", sign(&self.secret, joined.as_bytes()))
    }

    pub(super) fn challenge(&self) -> Response {
        let value = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\", scope=\"{}\"",
            self.public_url,
            default_scope()
        );
        (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_str(&value).unwrap(),
            )],
            "OAuth login required",
        )
            .into_response()
    }

    async fn login_page(&self, params: &AuthorizeParams, error: Option<&str>) -> Response {
        match self.login_fields(params).await {
            Ok(fields) => Html(login_html(&fields, error, &self.authorize_url())).into_response(),
            Err(err) => oauth_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        }
    }

    async fn login_fields(&self, params: &AuthorizeParams) -> Result<Vec<(String, String)>> {
        let challenge = random_token(32)?;
        let mut challenges = self.challenges.lock().await;
        let now = crate::util::now_secs();
        challenges.retain(|_, value| value.expires_at > now);
        challenges.insert(
            challenge.clone(),
            LoginChallenge::from_params(params, &self.resource_url, now + 300),
        );
        Ok(params.login_fields(&challenge))
    }

    async fn consume_challenge(
        &self,
        form: &AuthorizeForm,
        params: &AuthorizeParams,
    ) -> Result<String> {
        let Some(challenge) = self.challenges.lock().await.remove(&form.login_challenge) else {
            anyhow::bail!("unknown login challenge");
        };
        challenge.validate(params, &self.resource_url)?;
        Ok(form.login_challenge.clone())
    }

    fn pubkey_for_login(&self, form: &AuthorizeForm, challenge: &str) -> Result<String> {
        if let Some(nsec) = form
            .nsec
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return self.pubkey_for_nsec(nsec);
        }
        let pubkey = super::auth_nip07::pubkey_for_form(form, &self.public_url, challenge)?;
        self.ensure_whitelisted(&pubkey)
    }

    fn pubkey_for_nsec(&self, nsec: &str) -> Result<String> {
        let pubkey = Keys::parse(nsec.trim())
            .context("invalid nsec")?
            .public_key()
            .to_hex();
        self.ensure_whitelisted(&pubkey)
    }

    fn ensure_whitelisted(&self, pubkey: &str) -> Result<String> {
        if self
            .whitelisted
            .iter()
            .any(|key| normalize_pubkey(key) == pubkey)
        {
            Ok(pubkey.to_string())
        } else {
            anyhow::bail!("pubkey is not in whitelistedPubkeys")
        }
    }

    fn authorize_url(&self) -> String {
        format!("{}/oauth/authorize", self.public_url)
    }

    fn issue_token(&self, code: &AuthCode) -> Result<String> {
        // Access tokens do not expire: they are stateless, signed with the
        // persistent management key, so they remain valid across MCP server
        // restarts. Revocation, if ever needed, is via rotating that key.
        let claims = TokenClaims {
            iss: self.public_url.clone(),
            aud: code.resource.clone(),
            sub: code.pubkey.clone(),
            scope: code.scope.clone(),
            iat: crate::util::now_secs(),
        };
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
        let sig = sign(&self.secret, payload.as_bytes());
        Ok(format!("teo1.{payload}.{sig}"))
    }

    fn verify_token(&self, token: &str) -> Result<TokenClaims> {
        let parts = token.split('.').collect::<Vec<_>>();
        anyhow::ensure!(parts.len() == 3 && parts[0] == "teo1", "bad token");
        let expected = sign(&self.secret, parts[1].as_bytes());
        anyhow::ensure!(expected == parts[2], "bad signature");
        let claims: TokenClaims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1])?)?;
        anyhow::ensure!(claims.iss == self.public_url, "bad issuer");
        anyhow::ensure!(claims.aud == self.resource_url, "bad audience");
        Ok(claims)
    }
}

fn default_scope() -> String {
    SCOPES.join(" ")
}

#[cfg(test)]
#[path = "auth/tests.rs"]
mod tests;
