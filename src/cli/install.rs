//! Detect local agent harnesses and wire Mosaico hooks, plugins, and skills.

mod args;
mod config;
mod device_config;
mod goose;
mod hermes;
mod hooks;
mod io;
mod repair;
mod selection;
mod skill_api;
mod skills;

use anyhow::Result;
use config::Harness;
use owo_colors::OwoColorize;

use args::InstallOpts;
pub(super) use args::{setup, SetupArgs};
pub(super) use config::{harnesses, hook_entries, host_for_harness, OPENCODE_PLUGIN_TS};
pub(super) use device_config::ConfigRepair;
pub(super) use hooks::{is_installed, is_present, merge_hooks, migrate_codex_root_events};
pub(super) use io::{print_json_preview, read_json_or_default, write_json, write_text};
pub(super) use repair::{repair_device_config, repair_integration};
use selection::{detected_list, preflight_selection, resolve_selection};
pub(super) use skill_api::{
    repair_skill, skill_health, SkillHealth, SkillHealthState, SkillTargetHealth,
};

fn has_harness_installation() -> Result<bool> {
    Ok(harnesses()?.iter().any(is_installed))
}

fn print_setup_guide() {
    println!("Mosaico is not installed in any supported agent harness.\n");
    println!("Set it up with:\n\n  mosaico setup\n");
    println!(
        "This detects Claude Code, Codex, OpenCode, Grok, Goose, and Hermes and lets you choose integrations."
    );
    println!("Use `mosaico setup --all` to install every detected harness.");
}

/// Route a bare operator invocation to setup unless an integration is installed.
pub fn route_bare_invocation() -> Result<bool> {
    if has_harness_installation()? {
        Ok(true)
    } else {
        print_setup_guide();
        Ok(false)
    }
}

async fn install_with_opts(opts: InstallOpts) -> Result<()> {
    let all = harnesses()?;

    if opts.status {
        device_config::print_status()?;
        print_status(&all);
        skills::print_status()?;
        super::local_relay::print_status()?;
        return Ok(());
    }

    let selected = resolve_selection(&all, &opts)?;
    preflight_selection(&selected)?;
    let device = if opts.uninstall {
        None
    } else {
        Some(device_config::configure(&opts)?)
    };
    if selected.skill {
        skills::install(&opts)?;
    } else {
        println!("\n{}", "Skipping mosaico skill".dimmed());
    }

    if selected.harnesses.is_empty() && !opts.uninstall {
        println!(
            "No harness hooks selected. Detected: {}",
            detected_list(&all)
        );
    }

    let verb = if opts.uninstall {
        "Uninstalling from"
    } else {
        "Installing into"
    };
    let flag = if opts.dry_run { " (dry-run)" } else { "" };

    for h in selected.harnesses {
        println!("\n{} {}{flag}", verb.bold(), h.display.cyan().bold());
        match h.id {
            "claude-code" | "codex" | "grok" => install_json_harness(h, &opts, true)?,
            "opencode" => install_opencode(h, &opts, true)?,
            "goose" if opts.uninstall => goose::uninstall(h, &opts)?,
            "goose" => goose::install(h, &opts, true)?,
            "hermes" if opts.uninstall => hermes::uninstall(h, &opts)?,
            "hermes" => hermes::install(h, &opts, true)?,
            _ => {}
        }
    }

    if let Some(device) = device.as_ref() {
        if device.local_relay && device.start_local_relay {
            super::local_relay::start(
                device
                    .owner_pubkey
                    .as_deref()
                    .expect("local relay has owner"),
                opts.dry_run,
            )?;
        } else if !device.local_relay {
            super::local_relay::stop(opts.dry_run)?;
        }
    }

    if opts.dry_run {
        println!("\n{}", "(dry run; nothing was written)".dimmed());
    } else if !opts.uninstall {
        super::daemon_lifecycle::restart().await?;
        println!("\nSetup complete. Restart open harness sessions, then run `mosaico doctor`.");
    } else {
        println!("\nRemoved Mosaico-owned harness integrations and runtime skills.");
    }
    Ok(())
}

pub(super) async fn uninstall_everywhere(dry_run: bool) -> Result<()> {
    install_with_opts(InstallOpts::uninstall(dry_run)).await
}

fn print_status(all: &[Harness]) {
    println!("{}", "mosaico harness status".bold());
    for h in all {
        let detected = if h.detected {
            "detected".green().to_string()
        } else {
            "not detected".dimmed().to_string()
        };
        let installed = if is_installed(h) {
            "installed".green().to_string()
        } else {
            "-".dimmed().to_string()
        };
        println!(
            "  {:<12} {:<14} {:<10} {}",
            h.display.cyan(),
            detected,
            installed,
            h.config_path.display().to_string().dimmed()
        );
    }
}

fn install_json_harness(h: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    let mut root = read_json_or_default(&h.config_path)?;
    if h.id == "codex" {
        migrate_codex_root_events(&mut root);
    }
    let entries = hook_entries(h);
    let removed = merge_hooks(&mut root, &entries, host_for_harness(h), opts.uninstall);

    if opts.dry_run {
        let action = if opts.uninstall {
            format!("would remove {removed} hook group(s)")
        } else {
            format!("would write {} hook group(s)", entries.len())
        };
        if render {
            println!("  {action} in {}", h.config_path.display());
            print_json_preview(&root)?;
        }
        return Ok(());
    }

    write_json(&h.config_path, &root)?;
    if render {
        if opts.uninstall {
            println!("  removed {removed} hook group(s)");
        } else {
            println!("  wrote {}", h.config_path.display());
        }
    }
    Ok(())
}

fn install_opencode(h: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    if opts.uninstall {
        if !h.config_path.exists() {
            if render {
                println!("  nothing to remove");
            }
            return Ok(());
        }
        if opts.dry_run {
            if render {
                println!("  would remove {}", h.config_path.display());
            }
        } else {
            std::fs::remove_file(&h.config_path)?;
            if render {
                println!("  removed {}", h.config_path.display());
            }
        }
        return Ok(());
    }

    if opts.dry_run {
        if render {
            println!(
                "  would write {} ({} bytes)",
                h.config_path.display(),
                OPENCODE_PLUGIN_TS.len()
            );
        }
    } else {
        write_text(&h.config_path, OPENCODE_PLUGIN_TS)?;
        if render {
            println!("  wrote {}", h.config_path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
