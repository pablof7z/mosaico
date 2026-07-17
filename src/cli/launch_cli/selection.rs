use anyhow::Result;

pub(super) struct FreshAgentSelection {
    pub(super) slug: String,
}

pub(super) fn resolve_fresh_agent(
    requested: &str,
    cwd: &std::path::Path,
) -> Result<FreshAgentSelection> {
    let inventory = local_inventory(cwd)?;
    if let Some(selected) = inventory.find(requested) {
        return Ok(selection(selected));
    }

    let choices = inventory.profile_choices(requested);
    if choices.is_empty() {
        anyhow::bail!("no available agent named {requested:?}");
    }
    if !interactive_terminal() {
        anyhow::bail!(
            "agent {requested:?} is available from multiple harnesses; choose {}",
            choices
                .iter()
                .map(|choice| format!("`mosaico agents {}`", choice.slug))
                .collect::<Vec<_>>()
                .join(" or ")
        );
    }
    let labels = choices
        .iter()
        .map(|choice| choice.harness.agent_slug())
        .collect::<Vec<_>>();
    let index = dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt(format!("Select harness for {requested}"))
        .items(&labels)
        .default(0)
        .interact()?;
    Ok(selection(choices[index]))
}

fn interactive_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn selection(selected: &crate::agent_inventory::AvailableAgent) -> FreshAgentSelection {
    FreshAgentSelection {
        slug: selected.slug.clone(),
    }
}

fn local_inventory(cwd: &std::path::Path) -> Result<crate::agent_inventory::AgentInventory> {
    let harnesses = crate::harness::HarnessesConfig::load()?;
    let installed = crate::config::detect_available_harnesses()?;
    let catalog = crate::agent_catalog::AgentCatalog::discover(
        &crate::agent_catalog::DiscoveryRoots::installed()?,
        &[cwd.to_path_buf()],
    )?;
    Ok(crate::agent_inventory::AgentInventory::build(
        &crate::config::mosaico_home(),
        &installed,
        &harnesses,
        &catalog,
        Some(cwd),
    ))
}

#[cfg(test)]
#[path = "selection/tests.rs"]
mod tests;
