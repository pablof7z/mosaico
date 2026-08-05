//! The two trust inputs the MCP OAuth door must read *live*, never from a
//! snapshot taken when the server booted.
//!
//! `whitelistedPubkeys` answers "who may hold a credential here" and
//! `mcpRedirectOrigins` answers "where may an authorization code be delivered".
//! Both are operator decisions, and an operator who withdraws one expects the
//! withdrawal to take effect — so the door re-reads them per request rather
//! than caching what the config said at startup (mosaico#766: a pubkey removed
//! from `whitelistedPubkeys` kept working until the process was restarted, and
//! the only documented alternative was rotating `mosaicoPrivateKey`, which
//! destroys every group membership the backend owns).
//!
//! This is deliberately *not* `Config::load()`. That path spawns `hostname(1)`,
//! resolves the attachment directory and refuses a config with no relay — none
//! of which a token check should depend on, and the subprocess alone rules it
//! out for a per-request read.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct McpTrust {
    /// Human operators allowed to log in and to keep holding a token.
    pub(crate) operators: Vec<String>,
    /// Origins a client may register a redirect URI under. Loopback is always
    /// allowed and is not listed here.
    pub(crate) redirect_origins: Vec<String>,
}

#[derive(Deserialize)]
struct RawTrust {
    #[serde(default, rename = "whitelistedPubkeys")]
    whitelisted_pubkeys: Vec<String>,
    #[serde(default, rename = "mcpRedirectOrigins")]
    mcp_redirect_origins: Vec<String>,
}

pub(crate) fn from_json_str(body: &str) -> Result<McpTrust> {
    let raw: RawTrust = serde_json::from_str(body).context("parsing mosaico config json")?;
    Ok(McpTrust {
        operators: raw.whitelisted_pubkeys,
        redirect_origins: raw.mcp_redirect_origins,
    })
}

/// Read the current trust inputs from a specific config document.
///
/// Every failure is propagated rather than defaulted: the callers are
/// authorization checks, and an unreadable config must deny, not admit.
pub(crate) fn load_at(path: &Path) -> Result<McpTrust> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading {} for MCP trust", path.display()))?;
    from_json_str(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_lists_and_defaults_them_empty() {
        let trust = from_json_str(r#"{"whitelistedPubkeys":["aa"],"relays":[]}"#).unwrap();
        assert_eq!(trust.operators, vec!["aa".to_string()]);
        assert!(trust.redirect_origins.is_empty());

        let trust =
            from_json_str(r#"{"mcpRedirectOrigins":["https://claude.ai"],"relays":[]}"#).unwrap();
        assert!(trust.operators.is_empty());
        assert_eq!(
            trust.redirect_origins,
            vec!["https://claude.ai".to_string()]
        );
    }

    #[test]
    fn an_unreadable_config_is_an_error_not_an_empty_trust_set() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("config.json");
        assert!(load_at(&missing).is_err());
    }

    #[test]
    fn a_rewritten_config_is_observed_without_reconstructing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"whitelistedPubkeys":["aa","bb"]}"#).unwrap();
        assert_eq!(load_at(&path).unwrap().operators, vec!["aa", "bb"]);
        std::fs::write(&path, r#"{"whitelistedPubkeys":["aa"]}"#).unwrap();
        assert_eq!(load_at(&path).unwrap().operators, vec!["aa"]);
    }
}
