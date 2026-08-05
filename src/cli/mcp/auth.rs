//! The MCP OAuth door.
//!
//! Two rules hold this surface together, and mosaico#766 was what happened when
//! neither did:
//!
//! 1. **A code is only ever delivered somewhere a registration recorded.**
//!    `/oauth/register` writes a durable record ([`super::auth_clients`]) whose
//!    redirect targets an operator approved ([`super::auth_redirect`]), and
//!    `/oauth/authorize` refuses any `redirect_uri` that is not an exact match
//!    for one this `client_id` registered. PKCE cannot do this job: the party
//!    supplying `code_challenge` is the party asking for the code, so it binds
//!    the code to a challenge the requester picked and proves nothing about who
//!    the requester is.
//! 2. **A token never outlives the authority it was granted under.** Claims
//!    carry `exp`, and every verification re-reads `whitelistedPubkeys` from
//!    the config document instead of a snapshot taken at boot. Dropping an
//!    operator from that list ends their access on their next request, without
//!    touching `mosaicoPrivateKey` — rotating that key would also revoke
//!    tokens, but it is the daemon's NIP-29 management identity, so it would
//!    destroy every group membership and admin grant the backend owns.

use anyhow::Result;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

mod login;
mod token;

use super::auth_clients::ClientRegistry;
use super::auth_login_page::login_html;
use super::auth_support::{
    bearer, derive_key, normalize_pubkey, oauth_error, oauth_json_error, random_token,
    redirect_with_code, scope_allowed, sign, verify_signature,
};
use super::auth_types::{validate_token_request, AuthCode, LoginChallenge};
pub(super) use super::auth_types::{AuthorizeForm, AuthorizeParams, TokenForm};
use token::TOKEN_KEY_LABEL;

const SCOPES: &[&str] = &["mosaico:read", "mosaico:write"];

/// The client registry filename under mosaico's own writable home.
const CLIENT_REGISTRY_FILE: &str = "mcp-clients.json";

#[derive(Clone)]
pub(super) struct AuthState {
    public_url: String,
    resource_url: String,
    /// Keys actor correlation, and nothing else.
    actor_secret: Vec<u8>,
    /// Signs and verifies access tokens, and nothing else.
    token_key: Vec<u8>,
    /// The config document whose `whitelistedPubkeys` and `mcpRedirectOrigins`
    /// this door re-reads per request. The *path* is resolved once at startup;
    /// its *contents* are never cached.
    config_path: PathBuf,
    clients: ClientRegistry,
    codes: Arc<Mutex<HashMap<String, AuthCode>>>,
    challenges: Arc<Mutex<HashMap<String, LoginChallenge>>>,
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
        let secret = secret.into_bytes();
        let public_url = public_url.trim_end_matches('/').to_string();
        let resource_url = format!("{public_url}{resource_path}");
        Ok(Self {
            public_url,
            resource_url,
            token_key: derive_key(&secret, TOKEN_KEY_LABEL),
            actor_secret: secret,
            config_path: crate::config::config_path(),
            clients: ClientRegistry::open(crate::config::mosaico_home().join(CLIENT_REGISTRY_FILE)),
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
            "scopes_supported": SCOPES,
        })
    }

    pub(super) async fn authorize_page(&self, params: AuthorizeParams) -> Response {
        if let Err(err) = self.accept_authorize(&params) {
            return oauth_error(StatusCode::BAD_REQUEST, err.to_string());
        }
        self.login_page(&params, None).await
    }

    pub(super) async fn authorize_submit(&self, form: AuthorizeForm) -> Response {
        let params = form.params();
        if let Err(err) = self.accept_authorize(&params) {
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
        let now = crate::util::now_secs();
        let record = AuthCode {
            client_id: params.client_id.clone(),
            redirect_uri: params.redirect_uri.clone(),
            code_challenge: params.code_challenge.clone(),
            resource: params.resource_url(&self.resource_url),
            scope: params.scope.clone().unwrap_or_else(default_scope),
            pubkey,
            expires_at: now + 300,
        };
        let mut codes = self.codes.lock().await;
        // Redeemed codes are removed by `token`; expired ones are dropped here,
        // so an unredeemed code does not sit in memory for the life of the
        // process the way it used to.
        codes.retain(|_, code| code.expires_at > now);
        codes.insert(code.clone(), record);
        drop(codes);
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
        let trust = match self.trust() {
            Ok(trust) => trust,
            Err(err) => {
                return oauth_json_error("server_error", &format!("{err:#}"));
            }
        };
        match self.clients.register(&body, &trust.redirect_origins) {
            Ok(registration) => Json(registration).into_response(),
            Err(err) => oauth_json_error("invalid_redirect_uri", &format!("{err:#}")),
        }
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
        format!("mcp1_{}", sign(&self.actor_secret, joined.as_bytes()))
    }

    /// Everything `/oauth/authorize` checks before an operator is ever shown a
    /// login page. A failure here is answered on our own origin, never by
    /// redirecting to the target being refused.
    fn accept_authorize(&self, params: &AuthorizeParams) -> Result<()> {
        params.validate(&self.resource_url)?;
        self.clients
            .ensure_registered(&params.client_id, &params.redirect_uri)
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
}

fn default_scope() -> String {
    SCOPES.join(" ")
}

#[cfg(test)]
#[path = "auth/tests.rs"]
mod tests;
