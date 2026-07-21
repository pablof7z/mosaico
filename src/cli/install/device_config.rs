//! First-run bootstrap for `~/.mosaico/config.json` — the file the daemon
//! reads for `whitelistedPubkeys`/`relays`/`backendName` and the daemon-owned
//! `mosaicoPrivateKey`. The shared provisioning path also backfills a missing
//! management key, but fresh bootstrap should write a complete config.

use anyhow::Result;
use dialoguer::{Confirm, Input};
use nostr_sdk::prelude::Keys;
use owo_colors::OwoColorize;
use std::io::{self, IsTerminal as _};

/// Entry point called from `install_with_opts`. Skipped for `--uninstall`
/// (bootstrapping config while tearing down doesn't make sense); `--dry-run`
/// only reports what would happen.
pub(super) fn run_if_needed(opts: &super::args::InstallOpts) -> Result<()> {
    if opts.uninstall {
        return Ok(());
    }
    if opts.dry_run {
        note_if_missing();
        Ok(())
    } else {
        ensure_device_config()?;
        Ok(())
    }
}

/// Runs the interactive bootstrap when `config.json` is missing, and backfills
/// `mosaicoPrivateKey` when an existing config predates the backend-key split.
fn ensure_device_config() -> Result<()> {
    let path = crate::config::config_path();
    if path.exists() {
        crate::config::ensure_mosaico_private_key()?;
        return Ok(());
    }

    println!(
        "\n{} {}",
        "No device config found at".bold(),
        path.display().to_string().cyan()
    );
    println!("mosaico's daemon needs this file to start (whitelisted operator pubkeys, relays).");

    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        println!(
            "Not running in a terminal — skipping setup. The daemon won't start until \
             {} exists; re-run `mosaico install` in a terminal, or create it by hand.",
            path.display()
        );
        return Ok(());
    }

    let run_setup = Confirm::new()
        .with_prompt("Set it up now?")
        .default(true)
        .interact()?;
    if !run_setup {
        println!(
            "Skipped. The daemon won't start until {} exists — re-run \
             `mosaico install` to set it up later.",
            path.display()
        );
        return Ok(());
    }

    let pubkeys_raw: String = Input::new()
        .with_prompt("Whitelisted operator pubkey(s), hex, comma-separated (blank = none yet)")
        .allow_empty(true)
        .interact_text()?;
    let whitelisted_pubkeys: Vec<String> = pubkeys_raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let backend_name: String = Input::new()
        .with_prompt("Host label (backendName)")
        .default(crate::config::hostname())
        .interact_text()?;

    let relay: String = Input::new()
        .with_prompt("Relay")
        .default(crate::config::DEFAULT_RELAY.to_string())
        .interact_text()?;

    let doc = device_config_doc(
        whitelisted_pubkeys.clone(),
        relay,
        backend_name,
        crate::config::generate_mosaico_private_key(),
    );
    super::write_json(&path, &doc)?;
    println!("wrote {}", path.display());
    println!(
        "{}",
        "generated mosaicoPrivateKey for this backend".dimmed()
    );
    if whitelisted_pubkeys.is_empty() {
        println!(
            "{}",
            "note: no pubkeys whitelisted yet — add them to config.json before inviting operators."
                .dimmed()
        );
    }
    Ok(())
}

/// Non-interactive repair used by `mosaico doctor --fix`. Missing config is
/// created with safe local defaults and no trusted operators. Existing JSON is
/// preserved byte-for-field except for backfilling a missing backend key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::cli) enum ConfigRepair {
    Unchanged,
    Created,
    GeneratedManagementKey,
}

pub(super) fn repair_non_interactive() -> Result<ConfigRepair> {
    let path = crate::config::config_path();
    if path.exists() {
        let config = crate::config::Config::load()?;
        return match config.backend_nsec() {
            Some(secret) if Keys::parse(secret.trim()).is_ok() => Ok(ConfigRepair::Unchanged),
            Some(_) => anyhow::bail!(
                "{} contains an invalid mosaicoPrivateKey; refusing to rotate backend identity automatically",
                path.display()
            ),
            None => {
                crate::config::ensure_mosaico_private_key()?;
                Ok(ConfigRepair::GeneratedManagementKey)
            }
        };
    }

    let doc = device_config_doc(
        Vec::new(),
        crate::config::DEFAULT_RELAY.to_string(),
        crate::config::hostname(),
        crate::config::generate_mosaico_private_key(),
    );
    super::write_json(&path, &doc)?;
    Ok(ConfigRepair::Created)
}

fn device_config_doc(
    whitelisted_pubkeys: Vec<String>,
    relay: String,
    backend_name: String,
    mosaico_private_key: String,
) -> serde_json::Value {
    serde_json::json!({
        "whitelistedPubkeys": whitelisted_pubkeys,
        "relays": [relay],
        "backendName": backend_name,
        "mosaicoPrivateKey": mosaico_private_key,
    })
}

/// `--dry-run` variant: reports what would happen without prompting or writing.
fn note_if_missing() {
    let path = crate::config::config_path();
    if path.exists() {
        return;
    }
    println!(
        "\n{} {} (dry-run: would prompt to create it)",
        "No device config found at".bold(),
        path.display().to_string().cyan()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;

    #[test]
    fn fresh_device_config_includes_mosaico_private_key() {
        let doc = device_config_doc(
            vec!["operator".to_string()],
            "wss://relay.example".to_string(),
            "test-host".to_string(),
            "backend-secret".to_string(),
        );

        assert_eq!(
            doc.get("mosaicoPrivateKey").and_then(|v| v.as_str()),
            Some("backend-secret")
        );
    }

    #[test]
    fn doctor_repair_creates_safe_baseline_without_trusting_an_operator() {
        let temp = tempfile::tempdir().unwrap();
        let mosaico_home = temp.path().join(".mosaico");
        let mut env = EnvGuard::set("HOME", temp.path());
        env.set_var("MOSAICO_HOME", &mosaico_home);

        assert_eq!(repair_non_interactive().unwrap(), ConfigRepair::Created);

        let config = crate::config::Config::load().unwrap();
        assert!(config.whitelisted_pubkeys.is_empty());
        assert!(config
            .backend_nsec()
            .is_some_and(|secret| Keys::parse(secret).is_ok()));
        assert_eq!(config.relays, [crate::config::DEFAULT_RELAY]);
    }

    #[test]
    fn doctor_repair_refuses_to_rotate_invalid_backend_identity() {
        let temp = tempfile::tempdir().unwrap();
        let mosaico_home = temp.path().join(".mosaico");
        std::fs::create_dir_all(&mosaico_home).unwrap();
        let config_path = mosaico_home.join("config.json");
        let original = r#"{"mosaicoPrivateKey":"invalid","unknown":"preserved"}"#;
        std::fs::write(&config_path, original).unwrap();
        let mut env = EnvGuard::set("HOME", temp.path());
        env.set_var("MOSAICO_HOME", &mosaico_home);

        let error = repair_non_interactive().unwrap_err().to_string();

        assert!(error.contains("refusing to rotate backend identity"));
        assert_eq!(std::fs::read_to_string(config_path).unwrap(), original);
    }

    #[test]
    fn doctor_repair_backfills_only_a_missing_backend_key() {
        let temp = tempfile::tempdir().unwrap();
        let mosaico_home = temp.path().join(".mosaico");
        std::fs::create_dir_all(&mosaico_home).unwrap();
        let config_path = mosaico_home.join("config.json");
        std::fs::write(&config_path, r#"{"unknown":"preserved"}"#).unwrap();
        let mut env = EnvGuard::set("HOME", temp.path());
        env.set_var("MOSAICO_HOME", &mosaico_home);

        assert_eq!(
            repair_non_interactive().unwrap(),
            ConfigRepair::GeneratedManagementKey
        );
        let repaired: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
        assert_eq!(repaired["unknown"], "preserved");
        assert!(Keys::parse(repaired["mosaicoPrivateKey"].as_str().unwrap()).is_ok());
    }
}
