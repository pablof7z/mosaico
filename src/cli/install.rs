//! Detect local agent harnesses and wire tenex-edge hooks into each.
//! Mirrors the `pc install` surface: --all, --harness, --dry-run, --status,
//! and --uninstall.

mod config;
mod hooks;
mod io;

use anyhow::{bail, Result};
use config::Harness;
use dialoguer::MultiSelect;
use owo_colors::OwoColorize;
use std::io::{self as stdio, IsTerminal as _};

pub(super) use config::{harnesses, hook_entries, host_for_harness, OPENCODE_PLUGIN_TS};
pub(super) use hooks::{is_installed, merge_hooks, migrate_codex_root_events};
pub(super) use io::{print_json_preview, read_json_or_default, write_json, write_text};

pub(super) struct InstallOpts {
    pub all: bool,
    pub harness: Option<String>,
    pub dry_run: bool,
    pub status: bool,
    pub uninstall: bool,
}

pub(super) async fn install(opts: InstallOpts) -> Result<()> {
    let all = harnesses();

    if opts.status {
        print_status(&all);
        return Ok(());
    }

    let selected = resolve_selection(&all, &opts)?;
    if selected.is_empty() {
        println!("No harnesses selected. Detected: {}", detected_list(&all));
        return Ok(());
    }

    let verb = if opts.uninstall {
        "Uninstalling from"
    } else {
        "Installing into"
    };
    let flag = if opts.dry_run { " (dry-run)" } else { "" };

    for h in selected {
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

fn resolve_selection<'a>(all: &'a [Harness], opts: &InstallOpts) -> Result<Vec<&'a Harness>> {
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
        return Ok(all.iter().filter(|h| wanted.contains(&h.id)).collect());
    }

    if opts.all {
        return Ok(all.iter().filter(|h| h.detected).collect());
    }

    if stdio::stdin().is_terminal() && stdio::stdout().is_terminal() {
        return interactive_select(all);
    }

    Ok(all.iter().filter(|h| h.detected).collect())
}

fn interactive_select(all: &[Harness]) -> Result<Vec<&Harness>> {
    let labels: Vec<String> = all
        .iter()
        .map(|h| {
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
                "{:<14} {}{}  {}",
                h.display.cyan().bold(),
                status,
                installed,
                h.config_path.display().to_string().dimmed()
            )
        })
        .collect();

    let defaults: Vec<bool> = all.iter().map(|h| h.detected).collect();

    let chosen = MultiSelect::new()
        .with_prompt("Install tenex-edge hooks  (space to toggle, enter to apply)")
        .items(&labels)
        .defaults(&defaults)
        .interact()?;

    Ok(chosen.into_iter().map(|i| &all[i]).collect())
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
mod tests {
    use super::*;

    fn harness(id: &'static str, path: std::path::PathBuf) -> Harness {
        Harness {
            id,
            display: id,
            config_path: path,
            detected: true,
        }
    }

    #[test]
    fn merge_hooks_preserves_foreign_groups_and_replaces_ours() {
        let mut root = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [
                    {
                        "hooks": [{
                            "type": "command",
                            "command": "pc hook inject --harness codex",
                            "timeout": 30
                        }]
                    },
                    {
                        "hooks": [{
                            "type": "command",
                            "command": "tenex-edge hook --host codex --type old",
                            "timeout": 1
                        }]
                    }
                ]
            }
        });

        merge_hooks(&mut root, &config::codex_hook_entries(), "codex", false);

        let groups = root
            .pointer("/hooks/UserPromptSubmit")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().any(|g| {
            g.pointer("/hooks/0/command")
                .and_then(|v| v.as_str())
                .is_some_and(|c| c == "pc hook inject --harness codex")
        }));
        assert!(groups.iter().any(|g| {
            g.pointer("/hooks/0/command")
                .and_then(|v| v.as_str())
                .is_some_and(|c| c == "tenex-edge hook --host codex --type user-prompt-submit")
        }));
    }

    #[test]
    fn uninstall_removes_ours_and_empty_events_only() {
        let mut root = serde_json::json!({
            "hooks": {
                "Stop": [
                    {
                        "hooks": [{
                            "type": "command",
                            "command": "tenex-edge hook --host codex --type stop",
                            "timeout": 30
                        }]
                    }
                ],
                "UserPromptSubmit": [
                    {
                        "hooks": [{
                            "type": "command",
                            "command": "pc hook inject --harness codex",
                            "timeout": 30
                        }]
                    },
                    {
                        "hooks": [{
                            "type": "command",
                            "command": "tenex-edge hook --host codex --type user-prompt-submit",
                            "timeout": 30
                        }]
                    }
                ]
            }
        });

        let removed = merge_hooks(&mut root, &config::codex_hook_entries(), "codex", true);

        assert_eq!(removed, 2);
        assert!(root.pointer("/hooks/Stop").is_none());
        let groups = root
            .pointer("/hooks/UserPromptSubmit")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]
                .pointer("/hooks/0/command")
                .and_then(|v| v.as_str()),
            Some("pc hook inject --harness codex")
        );
    }

    #[test]
    fn codex_root_events_are_migrated_under_hooks() {
        let mut root = serde_json::json!({
            "Stop": [{
                "hooks": [{
                    "type": "command",
                    "command": "foreign stop",
                    "timeout": 1
                }]
            }],
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "existing stop",
                        "timeout": 1
                    }]
                }]
            }
        });

        migrate_codex_root_events(&mut root);

        assert!(root.get("Stop").is_none());
        let groups = root
            .pointer("/hooks/Stop")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn write_json_creates_parent_directories() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a/b/hooks.json");
        write_json(&path, &serde_json::json!({"hooks": {}})).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn status_detects_installed_codex_hooks() {
        let temp = tempfile::tempdir().unwrap();
        let h = harness("codex", temp.path().join("hooks.json"));
        let mut root = serde_json::json!({});
        merge_hooks(&mut root, &config::codex_hook_entries(), "codex", false);
        write_json(&h.config_path, &root).unwrap();

        assert!(is_installed(&h));
    }
}
