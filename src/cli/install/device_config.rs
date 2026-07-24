//! First-run and repeatable configuration for the device-owned Mosaico state.

use super::args::InstallOpts;
use anyhow::{bail, Result};
use dialoguer::Confirm;
use nostr::Keys;
use owo_colors::OwoColorize;
use serde_json::{json, Value};
use std::io::{self, IsTerminal as _};

mod document;
mod prompt;

use document::{
    apply_overrides, baseline_document, ensure_complete, has_overrides, missing_management_key,
    print_summary, read_document, summarize, summarize_document,
};
use prompt::edit_interactively;

pub(super) const LOCAL_RELAY_URL: &str = "ws://127.0.0.1:9888";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeviceSetup {
    pub local_relay: bool,
    pub start_local_relay: bool,
    pub owner_pubkey: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::cli) enum ConfigRepair {
    Unchanged,
    GeneratedManagementKey,
}

pub(super) fn repair_non_interactive() -> Result<ConfigRepair> {
    let path = crate::config::config_path();
    if !path.exists() {
        bail!(
            "{} does not exist; run `mosaico setup` and choose the bundled local relay or supply an existing relay URL",
            path.display()
        );
    }
    let mut doc = read_document(&path)?;
    match doc.get("mosaicoPrivateKey").and_then(Value::as_str) {
        Some(secret) if Keys::parse(secret.trim()).is_ok() => Ok(ConfigRepair::Unchanged),
        Some(_) => bail!(
            "{} contains an invalid mosaicoPrivateKey; refusing to rotate backend identity automatically",
            path.display()
        ),
        None => {
            doc.as_object_mut().expect("configuration is an object").insert(
                "mosaicoPrivateKey".into(),
                json!(crate::config::generate_mosaico_private_key()),
            );
            super::write_json(&path, &doc)?;
            Ok(ConfigRepair::GeneratedManagementKey)
        }
    }
}

/// Configure a missing device or update the supported fields of an existing
/// document. Unknown fields and secrets that the wizard does not own survive.
pub(super) fn configure(opts: &InstallOpts) -> Result<DeviceSetup> {
    let path = crate::config::config_path();
    let existed = path.exists();
    let mut doc = if existed {
        read_document(&path)?
    } else {
        baseline_document()
    };

    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let overrides = has_overrides(opts);
    let should_edit = if opts.dry_run || overrides || !existed {
        true
    } else if interactive {
        Confirm::new()
            .with_prompt("Review device configuration?")
            .default(true)
            .interact()?
    } else {
        false
    };

    if should_edit {
        if interactive && !overrides {
            edit_interactively(&mut doc)?;
        } else {
            apply_overrides(&mut doc, opts)?;
        }
    }
    ensure_complete(&mut doc)?;
    let setup = summarize(&doc, opts)?;

    if opts.dry_run {
        let action = if existed { "update" } else { "create" };
        println!(
            "\n{} {} ({action}; dry-run)",
            "Device config".bold(),
            path.display().to_string().cyan()
        );
        print_summary(&doc, &setup);
        return Ok(setup);
    }

    if !existed || should_edit || missing_management_key(&path)? {
        super::write_json(&path, &doc)?;
        println!("wrote {}", path.display());
    } else {
        println!("using existing device config at {}", path.display());
    }
    print_summary(&doc, &setup);
    Ok(setup)
}

pub(super) fn print_status() -> Result<()> {
    let path = crate::config::config_path();
    if !path.exists() {
        println!("device config   missing  {}", path.display());
        return Ok(());
    }
    let doc = read_document(&path)?;
    let setup = summarize_document(&doc)?;
    println!("device config   configured  {}", path.display());
    print_summary(&doc, &setup);
    Ok(())
}

#[cfg(test)]
#[path = "device_config/tests.rs"]
mod tests;
