use super::{Check, CheckStatus};
use nostr_sdk::prelude::Keys;
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
                .repair("run `mosaico doctor --fix` to create a missing baseline config"),
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
    key_ready
}
