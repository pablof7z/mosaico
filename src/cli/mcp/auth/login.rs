//! The operator login step inside `/oauth/authorize`.
//!
//! By the time any of this runs, the request has already been bound to a
//! registration: `AuthState::accept_authorize` established that this
//! `client_id` registered this exact `redirect_uri`. What remains is proving
//! *which* human is approving it, which the one-time login challenge and a
//! NIP-07 signature (or a pasted nsec) answer.

use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use nostr::Keys;

use super::{
    login_html, oauth_error, random_token, AuthState, AuthorizeForm, AuthorizeParams,
    LoginChallenge,
};

impl AuthState {
    pub(super) async fn login_page(
        &self,
        params: &AuthorizeParams,
        error: Option<&str>,
    ) -> Response {
        match self.login_fields(params).await {
            Ok(fields) => Html(login_html(
                &fields,
                error,
                &self.authorize_url(),
                &params.client_id,
                &params.redirect_uri,
            ))
            .into_response(),
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

    pub(super) async fn consume_challenge(
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

    pub(super) fn pubkey_for_login(&self, form: &AuthorizeForm, challenge: &str) -> Result<String> {
        if let Some(nsec) = form
            .nsec
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return self.pubkey_for_nsec(nsec);
        }
        let pubkey = super::super::auth_nip07::pubkey_for_form(form, &self.public_url, challenge)?;
        self.ensure_whitelisted(&pubkey)
    }

    fn pubkey_for_nsec(&self, nsec: &str) -> Result<String> {
        let pubkey = Keys::parse(nsec.trim())
            .context("invalid nsec")?
            .public_key()
            .to_hex();
        self.ensure_whitelisted(&pubkey)
    }

    pub(super) fn authorize_url(&self) -> String {
        format!("{}/oauth/authorize", self.public_url)
    }
}
