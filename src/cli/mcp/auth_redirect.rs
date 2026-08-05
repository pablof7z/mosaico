//! Where an authorization code may be delivered.
//!
//! Dynamic client registration is open by design — anything that can reach the
//! endpoint can ask for a `client_id`. That makes "the client registered this
//! redirect URI" a statement about the *requester*, not about anyone mosaico
//! trusts, so binding `redirect_uri` to `client_id` alone would still let an
//! attacker register their own callback, phish a whitelisted operator onto the
//! login page and collect the code (mosaico#766).
//!
//! So registration is constrained by an operator decision, exactly as login is:
//!
//! - **Loopback** targets are always acceptable. A code delivered to
//!   `127.0.0.1` reaches something already running on the operator's own
//!   machine, and RFC 8252 §7.3 requires the port to be free — an operator
//!   cannot pre-list a port the client picks at runtime.
//! - **Everything else** must be `https` on an origin listed in
//!   `mcpRedirectOrigins`. That is the operator saying "ChatGPT may hold codes
//!   for me", the sibling of `whitelistedPubkeys` saying which humans may.
//!
//! An origin, not a full URI: the operator approves *who* receives codes, and
//! the client keeps choosing its own callback path under that origin. The exact
//! URI still has to match what was registered — that check lives in
//! [`super::auth_clients`].

use anyhow::{Context, Result};
use url::{Host, Url};

/// Accept or refuse a redirect target, returning it in the exact spelling the
/// client must use at `/oauth/authorize`.
pub(super) fn accept(redirect_uri: &str, approved_origins: &[String]) -> Result<String> {
    let url = Url::parse(redirect_uri)
        .with_context(|| format!("redirect_uri must be an absolute URL: {redirect_uri}"))?;
    anyhow::ensure!(
        url.fragment().is_none(),
        "redirect_uri must not carry a fragment: {redirect_uri}"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "redirect_uri must not carry credentials: {redirect_uri}"
    );
    if is_loopback(&url) {
        return Ok(redirect_uri.to_string());
    }
    anyhow::ensure!(
        url.scheme() == "https",
        "a non-loopback redirect_uri must use https: {redirect_uri}"
    );
    let origin = origin_of(&url);
    anyhow::ensure!(
        approved_origins
            .iter()
            .filter_map(|approved| Url::parse(approved.trim()).ok())
            .any(|approved| origin_of(&approved) == origin),
        "{origin} is not an approved MCP redirect origin: add it to mcpRedirectOrigins in \
         mosaico's config, or use a loopback callback"
    );
    Ok(redirect_uri.to_string())
}

fn origin_of(url: &Url) -> String {
    url.origin().ascii_serialization()
}

/// `localhost` is accepted alongside the literal loopback addresses because the
/// MCP clients that run beside mosaico spell their callback that way. It is
/// resolved by the operator's own browser, on the operator's own machine.
fn is_loopback(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    match url.host() {
        Some(Host::Ipv4(addr)) => addr.is_loopback(),
        Some(Host::Ipv6(addr)) => addr.is_loopback(),
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved() -> Vec<String> {
        vec!["https://chatgpt.com".into(), "https://claude.ai".into()]
    }

    #[test]
    fn loopback_callbacks_need_no_operator_approval() {
        for uri in [
            "http://127.0.0.1:51763/oauth/callback",
            "http://localhost:8912/callback",
            "http://[::1]:4000/cb",
        ] {
            assert!(accept(uri, &[]).is_ok(), "{uri} should be accepted");
        }
    }

    #[test]
    fn an_unapproved_https_callback_cannot_be_registered() {
        let error = accept("https://evil.example/steal", &approved()).unwrap_err();
        assert!(
            error.to_string().contains("mcpRedirectOrigins"),
            "the refusal must name the operator control: {error}"
        );
    }

    #[test]
    fn an_approved_origin_admits_any_path_under_it_but_no_sibling_origin() {
        assert!(accept(
            "https://chatgpt.com/connector_platform_oauth_redirect",
            &approved()
        )
        .is_ok());
        assert!(accept("https://chatgpt.com.evil.example/cb", &approved()).is_err());
        assert!(accept("https://sub.chatgpt.com/cb", &approved()).is_err());
        assert!(accept("http://chatgpt.com/cb", &approved()).is_err());
    }

    #[test]
    fn a_port_is_part_of_the_origin() {
        let approved = vec!["https://client.example:8443".to_string()];
        assert!(accept("https://client.example:8443/cb", &approved).is_ok());
        assert!(accept("https://client.example/cb", &approved).is_err());
    }

    #[test]
    fn fragments_credentials_and_relative_targets_are_refused() {
        assert!(accept("https://chatgpt.com/cb#frag", &approved()).is_err());
        assert!(accept("https://user:pw@chatgpt.com/cb", &approved()).is_err());
        assert!(accept("/callback", &approved()).is_err());
        assert!(accept("javascript:alert(1)", &approved()).is_err());
    }
}
