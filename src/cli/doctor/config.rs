use super::{Check, CheckStatus};
use nostr::Keys;
use std::path::Path;

pub(super) fn inspect(path: &Path, checks: &mut Vec<Check>) -> bool {
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) => {
            checks.push(
                Check::new(
                    "config.document",
                    CheckStatus::Error,
                    format!("cannot read {}: {error}", path.display()),
                )
                .repair(
                    "run `mosaico setup --relay <ws-url>` with an externally operated NIP-29 relay",
                ),
            );
            return false;
        }
    };
    let config = match crate::config::Config::from_json_str(&body, &crate::config::hostname()) {
        Ok(config) => {
            checks.push(Check::new(
                "config.document",
                CheckStatus::Ok,
                format!("{} is valid JSON", path.display()),
            ));
            config
        }
        Err(error) => {
            checks.push(
                Check::new(
                    "config.document",
                    CheckStatus::Error,
                    format!("{} is invalid: {error:#}", path.display()),
                )
                .repair(
                    "repair the JSON without discarding its existing trust and identity fields",
                ),
            );
            return false;
        }
    };

    let relay_ready = if config.relays.is_empty() {
        checks.push(
            Check::new(
                "config.relays",
                CheckStatus::Error,
                "no fabric relay is configured",
            )
            .repair(
                "run `mosaico setup --relay <ws-url>` with an externally operated NIP-29 relay",
            ),
        );
        false
    } else {
        checks.push(Check::new(
            "config.relays",
            CheckStatus::Ok,
            format!("{} fabric relay(s) configured", config.relays.len()),
        ));
        true
    };

    let key_ready = match config.backend_nsec() {
        Some(secret) if Keys::parse(secret.trim()).is_ok() => {
            checks.push(Check::new(
                "config.management_key",
                CheckStatus::Ok,
                "mosaicoPrivateKey is usable",
            ));
            true
        }
        Some(_) => {
            checks.push(
                Check::new(
                    "config.management_key",
                    CheckStatus::Error,
                    "mosaicoPrivateKey is not a valid Nostr secret key",
                )
                .repair("replace the invalid key deliberately; doctor will not rotate backend identity automatically"),
            );
            false
        }
        None => {
            checks.push(
                Check::new(
                    "config.management_key",
                    CheckStatus::Error,
                    "mosaicoPrivateKey is missing",
                )
                .repair("run `mosaico doctor --fix` to generate the missing backend key"),
            );
            false
        }
    };

    checks.push(if config.whitelisted_pubkeys.is_empty() {
        Check::new(
            "config.operators",
            CheckStatus::Warning,
            "no operator pubkeys are whitelisted yet",
        )
        .repair("add the intended operator pubkey to whitelistedPubkeys before remote use")
    } else {
        Check::new(
            "config.operators",
            CheckStatus::Ok,
            format!(
                "{} operator pubkey(s) whitelisted",
                config.whitelisted_pubkeys.len()
            ),
        )
    });

    checks.push(mcp_redirect_origins_check(&body));
    relay_ready && key_ready
}

/// Which hosted MCP clients may be registered to receive an authorization code.
/// An empty list is not broken — loopback clients still work — but it is worth
/// reporting, because a hosted connector's registration will be refused and the
/// operator needs to know the control exists.
fn mcp_redirect_origins_check(body: &str) -> Check {
    let origins = match crate::config::mcp_trust::from_json_str(body) {
        Ok(trust) => trust.redirect_origins,
        Err(error) => {
            return Check::new(
                "config.mcp_redirect_origins",
                CheckStatus::Error,
                format!("cannot read mcpRedirectOrigins: {error:#}"),
            )
            .repair("repair the JSON without discarding its existing trust fields")
        }
    };
    if origins.is_empty() {
        Check::new(
            "config.mcp_redirect_origins",
            CheckStatus::Warning,
            "no MCP redirect origins are approved: only loopback clients can register",
        )
        .repair(
            "add each hosted MCP client's callback origin (e.g. https://chatgpt.com) to \
             mcpRedirectOrigins before connecting it",
        )
    } else {
        Check::new(
            "config.mcp_redirect_origins",
            CheckStatus::Ok,
            format!("{} MCP redirect origin(s) approved", origins.len()),
        )
    }
}
