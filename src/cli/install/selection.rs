use super::{is_installed, read_json_or_default, skills, Harness, InstallOpts};
use anyhow::{bail, Result};
use dialoguer::MultiSelect;
use owo_colors::OwoColorize;
use std::io::{self, IsTerminal as _};

pub(super) struct InstallSelection<'a> {
    pub skill: bool,
    pub harnesses: Vec<&'a Harness>,
    /// `Some` means synchronize the managed shell block to this exact set.
    /// `None` leaves shell profiles untouched.
    pub wrappers: Option<Vec<&'a Harness>>,
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
    if selected.wrappers.is_some() {
        super::shell::preflight_current_profile()?;
    }
    Ok(())
}

pub(super) fn resolve_selection<'a>(
    all: &'a [Harness],
    opts: &InstallOpts,
) -> Result<InstallSelection<'a>> {
    if opts.uninstall {
        let harnesses = match opts.harness.as_deref() {
            Some(id) => select_ids(all, id)?,
            None => all.iter().collect(),
        };
        return Ok(InstallSelection {
            skill: opts.harness.is_none(),
            harnesses,
            wrappers: None,
        });
    }
    if opts.harness.is_some() || opts.wrap.is_some() {
        let selected_ids = opts.harness.as_deref().or(opts.wrap.as_deref()).unwrap();
        let harnesses = select_ids(all, selected_ids)?;
        let wrappers = opts
            .wrap
            .as_deref()
            .map(|ids| select_ids(all, ids))
            .transpose()?;
        if let Some(wrappers) = wrappers.as_ref() {
            let outside = wrappers
                .iter()
                .filter(|wrapper| !harnesses.iter().any(|harness| harness.id == wrapper.id))
                .map(|wrapper| wrapper.id)
                .collect::<Vec<_>>();
            if !outside.is_empty() {
                bail!(
                    "wrapped harnesses must also be selected for setup: {}",
                    outside.join(", ")
                );
            }
        }
        return Ok(InstallSelection {
            skill: true,
            harnesses,
            wrappers,
        });
    }
    if opts.all {
        return Ok(InstallSelection {
            skill: true,
            harnesses: all.iter().filter(|harness| harness.detected).collect(),
            wrappers: None,
        });
    }
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return interactive_select(all);
    }
    println!("No harness integrations selected in non-interactive mode; pass --harness or --all.");
    Ok(InstallSelection {
        skill: true,
        harnesses: Vec::new(),
        wrappers: None,
    })
}

fn select_ids<'a>(all: &'a [Harness], ids: &str) -> Result<Vec<&'a Harness>> {
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
    Ok(all
        .iter()
        .filter(|harness| wanted.contains(&harness.id))
        .collect())
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
    let skill = chosen.contains(&0);
    let harnesses = chosen
        .into_iter()
        .filter_map(|index| index.checked_sub(1).map(|harness| &all[harness]))
        .collect::<Vec<_>>();
    let wrappers = interactive_wrappers(all, &harnesses)?;
    Ok(InstallSelection {
        skill,
        harnesses,
        wrappers,
    })
}

/// Offer a wrapper for each harness the operator just chose. `None` means this
/// machine has no profile Mosaico can own, so shell files stay untouched.
fn interactive_wrappers<'a>(
    all: &'a [Harness],
    selected: &[&'a Harness],
) -> Result<Option<Vec<&'a Harness>>> {
    if !super::shell::supported() {
        return Ok(None);
    }
    if selected.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let configured = super::shell::configured_wrappers(all)?;
    let labels = selected
        .iter()
        .map(|harness| {
            format!(
                "{:<18} {}",
                harness.display.cyan().bold(),
                super::shell::wrapper_preview(harness)
            )
        })
        .collect::<Vec<_>>();
    let defaults = selected
        .iter()
        .map(|harness| configured.contains(harness.id))
        .collect::<Vec<_>>();
    let chosen = MultiSelect::new()
        .with_prompt("Wrap harness commands through mosaico  (space to toggle, enter to apply)")
        .items(&labels)
        .defaults(&defaults)
        .interact()?;
    Ok(Some(
        chosen.into_iter().map(|index| selected[index]).collect(),
    ))
}

#[cfg(test)]
mod tests;
