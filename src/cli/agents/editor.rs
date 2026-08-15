use super::data::{harness_name, AgentKind, AgentRow};
use crate::harness::PresetsConfig;
use crate::session::Harness;
use anyhow::{bail, Result};
use dialoguer::{theme::ColorfulTheme, Select};
use std::io::IsTerminal as _;

/// Any `esc` press during this flow backs all the way out to the picker
/// without saving, rather than erroring or forcing the operator through the
/// remaining prompts.
pub(super) async fn edit(row: &AgentRow) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!("agent editing is interactive — run it in a terminal");
    }
    let theme = ColorfulTheme::default();
    let Some(harness) = select_harness(row, &theme)? else {
        return Ok(());
    };
    let Some(preset) = select_preset(harness, row.preset.as_deref(), &theme)? else {
        return Ok(());
    };
    let Some(per_session_key) = select_key_mode(row.per_session_key.unwrap_or(true), &theme)?
    else {
        return Ok(());
    };
    let profile = profile_for_save(row);
    let slug = persistable_slug(&row.slug);
    let preset_label = preset
        .as_deref()
        .map(|name| format!(" · preset {name}"))
        .unwrap_or_default();
    let saved = super::save_agent_config(
        &slug,
        harness.as_str(),
        profile,
        preset,
        Some(per_session_key),
    )
    .await?;
    println!(
        "{} {} · {}{}",
        if saved.created { "Created" } else { "Updated" },
        slug,
        harness_name(harness),
        preset_label
    );
    if slug != row.slug {
        println!(
            "  (native profile name {:?} isn't a valid agent slug — saved as {slug})",
            row.slug
        );
    }
    Ok(())
}

/// Some harnesses allow free-text profile names (e.g. "Ava Chen") that don't
/// satisfy the agent slug charset. Sanitize only when necessary so an
/// already-valid slug round-trips unchanged.
fn persistable_slug(slug: &str) -> String {
    if crate::identity::is_valid_slug(slug) {
        slug.to_string()
    } else {
        crate::slug::slugify(slug)
    }
}

fn profile_for_save(row: &AgentRow) -> Option<String> {
    (row.kind != AgentKind::NativeProfile)
        .then(|| row.profile.clone())
        .flatten()
}

fn select_harness(row: &AgentRow, theme: &ColorfulTheme) -> Result<Option<Harness>> {
    let available = Harness::ALL;
    let labels = available
        .iter()
        .map(|harness| harness_name(*harness))
        .collect::<Vec<_>>();
    let Some(choice) = Select::with_theme(theme)
        .with_prompt("Select harness")
        .items(&labels)
        .default(
            available
                .iter()
                .position(|harness| *harness == row.harness)
                .unwrap_or(0),
        )
        .interact_opt()?
    else {
        return Ok(None);
    };
    Ok(Some(available[choice]))
}

fn select_preset(
    harness: Harness,
    current: Option<&str>,
    theme: &ColorfulTheme,
) -> Result<Option<Option<String>>> {
    let mut names = PresetsConfig::load()?.names_for_harness(harness);
    names.sort();
    let mut labels = vec!["No preset".to_string()];
    labels.extend(names.iter().cloned());
    let default = current
        .and_then(|current| names.iter().position(|name| name == current))
        .map(|index| index + 1)
        .unwrap_or(0);
    let Some(choice) = Select::with_theme(theme)
        .with_prompt("Launch preset")
        .items(&labels)
        .default(default)
        .interact_opt()?
    else {
        return Ok(None);
    };
    Ok(Some(
        choice.checked_sub(1).map(|index| names[index].clone()),
    ))
}

fn select_key_mode(current_per_session: bool, theme: &ColorfulTheme) -> Result<Option<bool>> {
    let options = [
        "Per-session key — a fresh identity for every session",
        "Persistent key — reuse one identity across sessions",
    ];
    let Some(choice) = Select::with_theme(theme)
        .with_prompt("Agent identity")
        .items(&options)
        .default(usize::from(!current_per_session))
        .interact_opt()?
    else {
        return Ok(None);
    };
    Ok(Some(choice == 0))
}

#[cfg(test)]
#[path = "editor/tests.rs"]
mod tests;
