use super::{is_installed, read_json_or_default, Harness, InstallOpts};
use anyhow::{bail, Result};
use dialoguer::{Confirm, MultiSelect};
use owo_colors::OwoColorize;
use std::io::{self, IsTerminal as _};

pub(super) struct InstallSelection<'a> {
    pub skill: bool,
    pub harnesses: Vec<&'a Harness>,
}

impl InstallSelection<'_> {
    pub(super) fn display_names(&self) -> String {
        if self.harnesses.is_empty() {
            return "No agent apps".to_string();
        }
        self.harnesses
            .iter()
            .map(|harness| harness.display)
            .collect::<Vec<_>>()
            .join(", ")
    }
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
    let detected = all
        .iter()
        .filter(|harness| harness.detected)
        .collect::<Vec<_>>();
    if detected.is_empty() {
        println!(
            "\nNo supported agent apps were detected. Mosaico will configure the fabric and skill."
        );
        return Ok(InstallSelection {
            skill: true,
            harnesses: Vec::new(),
        });
    }

    let detected_names = detected
        .iter()
        .map(|harness| harness.display)
        .collect::<Vec<_>>()
        .join(", ");
    println!("\nAgent apps found: {detected_names}");
    if Confirm::new()
        .with_prompt("Connect all detected agent apps?")
        .default(true)
        .interact()?
    {
        return Ok(InstallSelection {
            skill: true,
            harnesses: detected,
        });
    }

    let labels = all.iter().map(|harness| {
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
        format!("{:<18} {status}{installed}", harness.display.cyan().bold())
    });
    let defaults = all
        .iter()
        .map(|harness| harness.detected)
        .collect::<Vec<_>>();
    let chosen = MultiSelect::new()
        .with_prompt("Choose agent apps  (space to toggle, enter to continue)")
        .items(&labels.collect::<Vec<_>>())
        .defaults(&defaults)
        .interact()?;
    Ok(InstallSelection {
        skill: true,
        harnesses: chosen.into_iter().map(|index| &all[index]).collect(),
    })
}

#[cfg(test)]
mod tests;
