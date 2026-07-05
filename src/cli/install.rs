//! Detect local agent harnesses and wire tenex-edge hooks into each.
//! Mirrors the `pc install` surface: --all, --harness, --dry-run, --status,
//! and --uninstall.

mod args;
mod config;
mod device_config;
mod hooks;
mod io;
mod skills;

use anyhow::{bail, Result};
use config::Harness;
use dialoguer::MultiSelect;
use owo_colors::OwoColorize;
use std::io::{self as stdio, IsTerminal as _};

use args::InstallOpts;
pub(super) use args::{install, InstallArgs};
pub(super) use config::{harnesses, hook_entries, host_for_harness, OPENCODE_PLUGIN_TS};
pub(super) use hooks::{is_installed, merge_hooks, migrate_codex_root_events};
pub(super) use io::{print_json_preview, read_json_or_default, write_json, write_text};

async fn install_with_opts(opts: InstallOpts) -> Result<()> {
    let all = harnesses()?;

    if opts.status {
        print_status(&all);
        skills::print_status()?;
        return Ok(());
    }

    device_config::run_if_needed(&opts)?;

    let selected = resolve_selection(&all, &opts)?;
    if selected.skill {
        skills::install(&opts)?;
    } else {
        println!("\n{}", "Skipping tenex-edge skill".dimmed());
    }

    if selected.harnesses.is_empty() {
        println!(
            "No harness hooks selected. Detected: {}",
            detected_list(&all)
        );
        return Ok(());
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
            "claude-code" | "codex" | "grok" => install_json_harness(h, &opts)?,
            "opencode" => install_opencode(h, &opts)?,
            _ => {}
        }
    }

    if opts.dry_run {
        println!("\n{}", "(dry run; nothing was written)".dimmed());
    } else if !opts.uninstall {
        println!("\nDone. Restart any open harness sessions to pick up the hooks.");
    }
    Ok(())
}

struct InstallSelection<'a> {
    skill: bool,
    harnesses: Vec<&'a Harness>,
}

fn print_status(all: &[Harness]) {
    println!("{}", "tenex-edge harness status".bold());
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

fn detected_list(all: &[Harness]) -> String {
    let detected = all
        .iter()
        .filter(|h| h.detected)
        .map(|h| h.id)
        .collect::<Vec<_>>();
    if detected.is_empty() {
        "(none)".to_string()
    } else {
        detected.join(", ")
    }
}

fn resolve_selection<'a>(all: &'a [Harness], opts: &InstallOpts) -> Result<InstallSelection<'a>> {
    if let Some(ids) = &opts.harness {
        let wanted = ids
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let unknown = wanted
            .iter()
            .copied()
            .filter(|id| !all.iter().any(|h| h.id == *id))
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            bail!(
                "unknown harness id(s): {}. Known: {}",
                unknown.join(", "),
                all.iter().map(|h| h.id).collect::<Vec<_>>().join(", ")
            );
        }
        return Ok(InstallSelection {
            skill: true,
            harnesses: all.iter().filter(|h| wanted.contains(&h.id)).collect(),
        });
    }

    if opts.all {
        return Ok(InstallSelection {
            skill: true,
            harnesses: all.iter().filter(|h| h.detected).collect(),
        });
    }

    if stdio::stdin().is_terminal() && stdio::stdout().is_terminal() {
        return interactive_select(all);
    }

    Ok(InstallSelection {
        skill: true,
        harnesses: all.iter().filter(|h| h.detected).collect(),
    })
}

fn interactive_select(all: &[Harness]) -> Result<InstallSelection<'_>> {
    let mut labels = vec![skills::selection_label()?];
    labels.extend(all.iter().map(|h| {
        let status = if h.detected {
            "detected".green().to_string()
        } else {
            "not detected".dimmed().to_string()
        };
        let installed = if is_installed(h) {
            format!("  {}", "installed".green())
        } else {
            String::new()
        };
        format!(
            "{:<18} {}{}  {}",
            h.display.cyan().bold(),
            status,
            installed,
            h.config_path.display().to_string().dimmed()
        )
    }));

    let mut defaults = vec![true];
    defaults.extend(all.iter().map(|h| h.detected));

    let chosen = MultiSelect::new()
        .with_prompt("Install tenex-edge components  (space to toggle, enter to apply)")
        .items(&labels)
        .defaults(&defaults)
        .interact()?;

    Ok(InstallSelection {
        skill: chosen.contains(&0),
        harnesses: chosen
            .into_iter()
            .filter_map(|i| i.checked_sub(1).map(|harness| &all[harness]))
            .collect(),
    })
}

fn install_json_harness(h: &Harness, opts: &InstallOpts) -> Result<()> {
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
        println!("  {action} in {}", h.config_path.display());
        print_json_preview(&root)?;
        return Ok(());
    }

    write_json(&h.config_path, &root)?;
    if opts.uninstall {
        println!("  removed {removed} hook group(s)");
    } else {
        println!("  wrote {}", h.config_path.display());
    }
    Ok(())
}

fn install_opencode(h: &Harness, opts: &InstallOpts) -> Result<()> {
    if opts.uninstall {
        if !h.config_path.exists() {
            println!("  nothing to remove");
            return Ok(());
        }
        if opts.dry_run {
            println!("  would remove {}", h.config_path.display());
        } else {
            std::fs::remove_file(&h.config_path)?;
            println!("  removed {}", h.config_path.display());
        }
        return Ok(());
    }

    if opts.dry_run {
        println!(
            "  would write {} ({} bytes)",
            h.config_path.display(),
            OPENCODE_PLUGIN_TS.len()
        );
    } else {
        write_text(&h.config_path, OPENCODE_PLUGIN_TS)?;
        println!("  wrote {}", h.config_path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
