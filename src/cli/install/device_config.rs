//! First-run and repeatable configuration for the device-owned Mosaico state.

use super::args::InstallOpts;
use anyhow::{bail, Result};
use dialoguer::Confirm;
use nostr_sdk::Keys;
use owo_colors::OwoColorize;
use serde_json::{json, Value};
use std::io::{self, IsTerminal as _};
use std::path::PathBuf;

mod document;
mod prompt;

use document::{
    apply_overrides, baseline_document, ensure_complete, has_overrides, missing_management_key,
    print_summary, read_document, summarize, summarize_document,
};
use prompt::{edit_interactively, onboard_interactively};

pub(super) const LOCAL_RELAY_URL: &str = "ws://127.0.0.1:9888";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeviceSetup {
    pub local_relay: bool,
    pub start_local_relay: bool,
    pub owner_pubkey: Option<String>,
}

pub(super) struct DevicePlan {
    path: PathBuf,
    document: Value,
    setup: DeviceSetup,
    existed: bool,
    should_write: bool,
}

impl DevicePlan {
    pub(super) fn setup(&self) -> &DeviceSetup {
        &self.setup
    }

    pub(super) fn print_review(&self) {
        let config = crate::config::Config::from_json_str(
            &self.document.to_string(),
            &crate::config::hostname(),
        )
        .expect("validated configuration");
        let fabric = if self.setup.local_relay {
            "Private fabric on this computer".to_string()
        } else {
            format!("Existing fabric via {}", config.relays.join(", "))
        };
        let operator = if config.user_nsec().is_some() {
            "Local operator identity ready".to_string()
        } else {
            match config.whitelisted_pubkeys.len() {
                0 => "No human operator identity".to_string(),
                1 => "One existing operator identity".to_string(),
                count => format!("{count} existing operator identities"),
            }
        };

        println!("  {:<14} {fabric}", "Fabric");
        println!("  {:<14} {}", "This device", config.host);
        println!("  {:<14} {operator}", "You");
    }

    pub(super) fn apply(&self, opts: &InstallOpts) -> Result<()> {
        if opts.dry_run {
            let action = if self.existed { "update" } else { "create" };
            println!(
                "\n{} {} ({action}; dry-run)",
                "Device config".bold(),
                self.path.display().to_string().cyan()
            );
            print_summary(&self.document, &self.setup);
            return Ok(());
        }

        if self.should_write {
            super::write_json(&self.path, &self.document)?;
            println!("wrote {}", self.path.display());
        } else {
            println!("using existing device config at {}", self.path.display());
        }
        print_summary(&self.document, &self.setup);
        Ok(())
    }
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

/// Prepare a missing device or update the supported fields of an existing
/// document without writing. Unknown fields and unedited secrets survive.
pub(super) fn prepare(opts: &InstallOpts) -> Result<DevicePlan> {
    let path = crate::config::config_path();
    let existed = path.exists();
    let mut doc = if existed {
        read_document(&path)?
    } else {
        baseline_document()
    };

    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let overrides = has_overrides(opts);
    let should_edit = if overrides || !existed {
        true
    } else if interactive {
        println!(
            "\nMosaico is already configured for this device at {}.",
            path.display()
        );
        Confirm::new()
            .with_prompt("Change fabric or identity settings?")
            .default(false)
            .interact()?
    } else {
        false
    };

    if should_edit {
        if interactive && !overrides {
            if existed {
                edit_interactively(&mut doc)?;
            } else {
                onboard_interactively(&mut doc)?;
            }
        } else {
            apply_overrides(&mut doc, opts)?;
        }
    }
    ensure_complete(&mut doc)?;
    let setup = summarize(&doc, opts)?;
    let should_write = !existed || should_edit || missing_management_key(&path)?;
    Ok(DevicePlan {
        path,
        document: doc,
        setup,
        existed,
        should_write,
    })
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
