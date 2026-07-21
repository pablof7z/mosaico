use super::{is_installed, read_json_or_default, skills, Harness, InstallOpts};
use anyhow::{bail, Result};
use dialoguer::MultiSelect;
use owo_colors::OwoColorize;
use std::io::{self, IsTerminal as _};

pub(super) struct InstallSelection<'a> {
    pub skill: bool,
    pub harnesses: Vec<&'a Harness>,
}

pub(super) fn preflight_selection(selected: &InstallSelection<'_>) -> Result<()> {
    for harness in &selected.harnesses {
        if !matches!(harness.id, "claude-code" | "codex" | "grok") {
            continue;
        }
        let root = read_json_or_default(&harness.config_path)?;
        let Some(root) = root.as_object() else {
            bail!(
                "{} must contain a JSON object; refusing to overwrite it",
                harness.config_path.display()
            );
        };
        if let Some(hooks) = root.get("hooks") {
            let Some(hooks) = hooks.as_object() else {
                bail!(
                    "{}.hooks must be a JSON object; refusing to overwrite it",
                    harness.config_path.display()
                );
            };
            for (event, groups) in hooks {
                if !groups.is_array() {
                    bail!(
                        "{}.hooks.{event} must be an array; refusing to overwrite it",
                        harness.config_path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

pub(super) fn resolve_selection<'a>(
    all: &'a [Harness],
    opts: &InstallOpts,
) -> Result<InstallSelection<'a>> {
    if opts.uninstall {
        return Ok(InstallSelection {
            skill: true,
            harnesses: all.iter().collect(),
        });
    }
    if let Some(ids) = &opts.harness {
        let wanted = ids
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let unknown = wanted
            .iter()
            .copied()
            .filter(|id| !all.iter().any(|harness| harness.id == *id))
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            bail!(
                "unknown harness id(s): {}. Known: {}",
                unknown.join(", "),
                all.iter()
                    .map(|harness| harness.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return Ok(InstallSelection {
            skill: true,
            harnesses: all
                .iter()
                .filter(|harness| wanted.contains(&harness.id))
                .collect(),
        });
    }
    if opts.all {
        return Ok(InstallSelection {
            skill: true,
            harnesses: all.iter().filter(|harness| harness.detected).collect(),
        });
    }
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return interactive_select(all);
    }
    println!("No harness integrations selected in non-interactive mode; pass --harness or --all.");
    Ok(InstallSelection {
        skill: true,
        harnesses: Vec::new(),
    })
}

pub(super) fn detected_list(all: &[Harness]) -> String {
    let detected = all
        .iter()
        .filter(|harness| harness.detected)
        .map(|harness| harness.id)
        .collect::<Vec<_>>();
    if detected.is_empty() {
        "(none)".to_string()
    } else {
        detected.join(", ")
    }
}

fn interactive_select(all: &[Harness]) -> Result<InstallSelection<'_>> {
    let mut labels = vec![skills::selection_label()?];
    labels.extend(all.iter().map(|harness| {
        let status = if harness.detected {
            "detected".green().to_string()
        } else {
            "not detected".dimmed().to_string()
        };
        let installed = if is_installed(harness) {
            format!("  {}", "installed".green())
        } else {
            String::new()
        };
        format!(
            "{:<18} {}{}  {}",
            harness.display.cyan().bold(),
            status,
            installed,
            harness.config_path.display().to_string().dimmed()
        )
    }));
    let mut defaults = vec![true];
    defaults.extend(all.iter().map(|harness| harness.detected));
    let chosen = MultiSelect::new()
        .with_prompt("Install mosaico components  (space to toggle, enter to apply)")
        .items(&labels)
        .defaults(&defaults)
        .interact()?;
    Ok(InstallSelection {
        skill: chosen.contains(&0),
        harnesses: chosen
            .into_iter()
            .filter_map(|index| index.checked_sub(1).map(|harness| &all[harness]))
            .collect(),
    })
}

#[cfg(test)]
mod tests;
