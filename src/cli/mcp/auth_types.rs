use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(super) struct AuthCode {
    pub(super) client_id: String,
    pub(super) redirect_uri: String,
    pub(super) code_challenge: String,
    pub(super) resource: String,
    pub(super) scope: String,
    pub(super) pubkey: String,
    pub(super) expires_at: u64,
}

#[derive(Clone)]
pub(super) struct LoginChallenge {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    resource: String,
    scope: Option<String>,
    pub(super) expires_at: u64,
}

#[derive(Deserialize, Clone)]
pub(super) struct AuthorizeParams {
    response_type: String,
    pub(super) client_id: String,
    pub(super) redirect_uri: String,
    pub(super) state: Option<String>,
    pub(super) code_challenge: String,
    code_challenge_method: String,
    resource: Option<String>,
    pub(super) scope: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct AuthorizeForm {
    pub(super) nsec: Option<String>,
    pub(super) nip07_pubkey: Option<String>,
    pub(super) nip07_event: Option<String>,
    pub(super) login_challenge: String,
    response_type: String,
    client_id: String,
    redirect_uri: String,
    state: Option<String>,
    code_challenge: String,
    code_challenge_method: String,
    resource: Option<String>,
    scope: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct TokenForm {
    pub(super) grant_type: String,
    pub(super) code: String,
    pub(super) redirect_uri: String,
    pub(super) client_id: String,
    code_verifier: String,
    resource: Option<String>,
}

impl AuthorizeParams {
    /// The protocol-shape checks, and only those.
    ///
    /// `redirect_uri` is deliberately not judged here. Nothing carried by the
    /// request can say whether a redirect target is legitimate — only the
    /// registration recorded for this `client_id` can, and that check lives in
    /// `AuthState::accept_authorize`. Parsing the URI here was what made it
    /// look validated when nothing had validated it.
    pub(super) fn validate(&self, default_resource: &str) -> Result<()> {
        anyhow::ensure!(self.response_type == "code", "response_type must be code");
        anyhow::ensure!(
            self.code_challenge_method == "S256",
            "PKCE S256 is required"
        );
        anyhow::ensure!(
            self.resource_url(default_resource) == default_resource,
            "resource does not match this MCP server"
        );
        Ok(())
    }

    pub(super) fn resource_url(&self, default_resource: &str) -> String {
        self.resource
            .clone()
            .unwrap_or_else(|| default_resource.to_string())
    }

    pub(super) fn login_fields(&self, challenge: &str) -> Vec<(String, String)> {
        vec![
            ("login_challenge".into(), challenge.into()),
            ("response_type".into(), self.response_type.clone()),
            ("client_id".into(), self.client_id.clone()),
            ("redirect_uri".into(), self.redirect_uri.clone()),
            ("state".into(), self.state.clone().unwrap_or_default()),
            ("code_challenge".into(), self.code_challenge.clone()),
            (
                "code_challenge_method".into(),
                self.code_challenge_method.clone(),
            ),
            ("resource".into(), self.resource.clone().unwrap_or_default()),
            ("scope".into(), self.scope.clone().unwrap_or_default()),
        ]
    }
}

impl LoginChallenge {
    pub(super) fn from_params(
        params: &AuthorizeParams,
        default_resource: &str,
        expires_at: u64,
    ) -> Self {
        Self {
            client_id: params.client_id.clone(),
            redirect_uri: params.redirect_uri.clone(),
            code_challenge: params.code_challenge.clone(),
            resource: params.resource_url(default_resource),
            scope: params.scope.clone(),
            expires_at,
        }
    }

    pub(super) fn validate(&self, params: &AuthorizeParams, default_resource: &str) -> Result<()> {
        anyhow::ensure!(
            self.expires_at > crate::util::now_secs(),
            "login challenge expired"
        );
        anyhow::ensure!(self.client_id == params.client_id, "client_id mismatch");
        anyhow::ensure!(
            self.redirect_uri == params.redirect_uri,
            "redirect_uri mismatch"
        );
        anyhow::ensure!(
            self.code_challenge == params.code_challenge,
            "code_challenge mismatch"
        );
        anyhow::ensure!(
            self.resource == params.resource_url(default_resource),
            "resource mismatch"
        );
        anyhow::ensure!(self.scope == params.scope, "scope mismatch");
        Ok(())
    }
}

impl AuthorizeForm {
    pub(super) fn params(&self) -> AuthorizeParams {
        AuthorizeParams {
            response_type: self.response_type.clone(),
            client_id: self.client_id.clone(),
            redirect_uri: self.redirect_uri.clone(),
            state: self.state.clone(),
            code_challenge: self.code_challenge.clone(),
            code_challenge_method: self.code_challenge_method.clone(),
            resource: self.resource.clone(),
            scope: self.scope.clone(),
        }
    }
}

pub(super) fn validate_token_request(
    form: &TokenForm,
    code: &AuthCode,
    default_resource: &str,
) -> Result<()> {
    anyhow::ensure!(code.expires_at > crate::util::now_secs(), "code expired");
    anyhow::ensure!(form.client_id == code.client_id, "client_id mismatch");
    anyhow::ensure!(
        form.redirect_uri == code.redirect_uri,
        "redirect_uri mismatch"
    );
    anyhow::ensure!(
        form.resource.as_deref().unwrap_or(default_resource) == code.resource,
        "resource mismatch"
    );
    let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(form.code_verifier.as_bytes()));
    anyhow::ensure!(expected == code.code_challenge, "PKCE verifier mismatch");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(resource: Option<&str>) -> AuthorizeParams {
        AuthorizeParams {
            response_type: "code".into(),
            client_id: "https://client.example/client.json".into(),
            redirect_uri: "https://client.example/callback".into(),
            state: Some("state".into()),
            code_challenge: "challenge".into(),
            code_challenge_method: "S256".into(),
            resource: resource.map(str::to_string),
            scope: Some("mosaico:read".into()),
        }
    }

    #[test]
    fn authorization_accepts_canonical_mcp_resource() {
        assert!(params(Some("https://mosaico.example/mcp"))
            .validate("https://mosaico.example/mcp")
            .is_ok());
    }

    #[test]
    fn authorization_rejects_authorization_server_origin_as_resource() {
        let error = params(Some("https://mosaico.example"))
            .validate("https://mosaico.example/mcp")
            .expect_err("origin must not identify the path-hosted MCP resource");
        assert_eq!(error.to_string(), "resource does not match this MCP server");
    }
}
