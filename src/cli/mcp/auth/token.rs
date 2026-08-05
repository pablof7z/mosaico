//! Access tokens: what they claim, how long they claim it for, and the
//! authority question re-asked every time one is presented.
//!
//! Tokens stay stateless — the server keeps no session table, so it survives
//! its own restart — but statelessness used to mean unrevokable (mosaico#766).
//! Two things fix that without a session table:
//!
//! - `exp` bounds how long a token that leaked out of a client's storage stays
//!   useful, which no authority check can do.
//! - Re-reading `whitelistedPubkeys` at verification time makes removing an
//!   operator an actual revocation, which no expiry can do.
//!
//! Both are needed, for those different reasons. The alternative the old code
//! documented — rotating the signing key — does revoke every token at once, but
//! that key is `mosaicoPrivateKey`, the daemon's NIP-29 management identity, so
//! it would also destroy every group membership and admin grant the backend
//! owns. Revocation must not cost that.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};

use super::{normalize_pubkey, sign, verify_signature, AuthCode, AuthState};

/// One hour, the lifetime the operator-facing MCP setup reference has always
/// documented. It need not be shorter: the whitelist re-check below, not this
/// number, is what ends an operator's access.
pub(super) const ACCESS_TOKEN_TTL_SECS: u64 = 3600;

/// Names the sole purpose of the token-signing key, so that it is not also the
/// key that pseudonymises actor correlation.
pub(super) const TOKEN_KEY_LABEL: &[u8] = b"mosaico/mcp/oauth-access-token/v1";

#[derive(Deserialize, Serialize)]
pub(super) struct TokenClaims {
    pub(super) iss: String,
    pub(super) aud: String,
    pub(super) sub: String,
    pub(super) scope: String,
    pub(super) iat: u64,
    pub(super) exp: u64,
}

impl AuthState {
    /// The operator's live decisions about who may hold a credential and where
    /// a code may be delivered. Read per request, never cached — see
    /// [`crate::config::mcp_trust`].
    pub(super) fn trust(&self) -> Result<crate::config::mcp_trust::McpTrust> {
        crate::config::mcp_trust::load_at(&self.config_path)
    }

    /// The single authority question this door asks, at login *and* at every
    /// later token verification. Reading the config document each time is what
    /// makes removing a pubkey from `whitelistedPubkeys` a revocation rather
    /// than a note that takes effect at the next restart.
    pub(super) fn ensure_whitelisted(&self, pubkey: &str) -> Result<String> {
        let trust = self.trust()?;
        if trust
            .operators
            .iter()
            .any(|key| normalize_pubkey(key) == pubkey)
        {
            Ok(pubkey.to_string())
        } else {
            anyhow::bail!("pubkey is not in whitelistedPubkeys")
        }
    }

    pub(super) fn issue_token(&self, code: &AuthCode) -> Result<String> {
        let issued_at = crate::util::now_secs();
        let claims = TokenClaims {
            iss: self.public_url.clone(),
            aud: code.resource.clone(),
            sub: code.pubkey.clone(),
            scope: code.scope.clone(),
            iat: issued_at,
            exp: issued_at + ACCESS_TOKEN_TTL_SECS,
        };
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
        let sig = sign(&self.token_key, payload.as_bytes());
        Ok(format!("teo1.{payload}.{sig}"))
    }

    pub(super) fn verify_token(&self, token: &str) -> Result<TokenClaims> {
        let parts = token.split('.').collect::<Vec<_>>();
        anyhow::ensure!(parts.len() == 3 && parts[0] == "teo1", "bad token");
        anyhow::ensure!(
            verify_signature(&self.token_key, parts[1].as_bytes(), parts[2]),
            "bad signature"
        );
        let claims: TokenClaims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1])?)?;
        anyhow::ensure!(claims.iss == self.public_url, "bad issuer");
        anyhow::ensure!(claims.aud == self.resource_url, "bad audience");
        anyhow::ensure!(claims.exp > crate::util::now_secs(), "token expired");
        self.ensure_whitelisted(&claims.sub)
            .context("token subject no longer holds an operator whitelist entry")?;
        Ok(claims)
    }
}
