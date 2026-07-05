//! First-run bootstrap for `~/.tenex-edge/config.json` — the file the daemon
//! reads for `whitelistedPubkeys`/`relays`/`backendName`. Nothing else in the
//! codebase writes it (see `crate::config::Config::load`), so a fresh machine
//! has no daemon-startable config until something creates it. Before this,
//! `tenex-edge install` never touched it, so a user could run the full install
//! flow, believe they were set up, and only discover the daemon refuses to
//! start (`loading config: reading .../config.json: No such file or
//! directory`) later, from `tenex-edge launch`.

use anyhow::Result;
use dialoguer::{Confirm, Input};
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
        ensure_device_config()
    }
}

/// Runs the interactive bootstrap when `config.json` is missing. No-op if it
/// already exists. Never fails the surrounding `install` command — worst case
/// it prints guidance and leaves the file absent for the user to create later.
fn ensure_device_config() -> Result<()> {
    let path = crate::config::config_path();
    if path.exists() {
        return Ok(());
    }

    println!(
        "\n{} {}",
        "No device config found at".bold(),
        path.display().to_string().cyan()
    );
    println!(
        "tenex-edge's daemon needs this file to start (whitelisted operator pubkeys, relays)."
    );

    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        println!(
            "Not running in a terminal — skipping setup. The daemon won't start until \
             {} exists; re-run `tenex-edge install` in a terminal, or create it by hand.",
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
             `tenex-edge install` to set it up later.",
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

    let doc = serde_json::json!({
        "whitelistedPubkeys": whitelisted_pubkeys,
        "relays": [relay],
        "backendName": backend_name,
    });
    super::write_json(&path, &doc)?;
    println!("wrote {}", path.display());
    if whitelisted_pubkeys.is_empty() {
        println!(
            "{}",
            "note: no pubkeys whitelisted yet — add them to config.json before inviting operators."
                .dimmed()
        );
    }
    Ok(())
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
