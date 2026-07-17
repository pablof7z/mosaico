use anyhow::Result;

mod usage;
use usage::{fetch_agent_usage, ordered_agents, usage_for, AgentUsageMap};

use crate::cli::interactive::agent_picker::{
    self, AgentPickerRow, AgentProvenance, PickerAction, PickerMode,
};

pub(super) struct FreshAgentSelection {
    pub(super) slug: String,
}

pub(super) async fn select_available() -> Result<Option<String>> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let inventory = local_inventory(&cwd)?;
    if inventory.agents.is_empty() {
        println!("No available agents.");
        for failure in inventory.failures {
            eprintln!("- unavailable: {failure}");
        }
        return Ok(None);
    }
    let now = crate::util::now_secs();
    let usage = fetch_agent_usage(now).await?;
    let agents = ordered_agents(&inventory, &usage);
    let rows = menu_rows(&agents, &usage, now);
    if !interactive_terminal() {
        println!("Available launch targets:");
        for row in rows {
            println!("- {}", row.plain());
        }
        return Ok(None);
    }

    match agent_picker::select(rows, PickerMode::Launch)? {
        PickerAction::Launch(index) => Ok(Some(agents[index].slug.clone())),
        PickerAction::Cancel => Ok(None),
        PickerAction::Edit(_) | PickerAction::Delete(_) => unreachable!("launch picker actions"),
    }
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
                .map(|choice| format!("`mosaico launch {}`", choice.slug))
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

fn menu_rows(
    agents: &[&crate::agent_inventory::AvailableAgent],
    usage: &AgentUsageMap,
    now: u64,
) -> Vec<AgentPickerRow> {
    agents
        .iter()
        .map(|agent| menu_row(agent, usage, now))
        .collect()
}

fn menu_row(
    agent: &crate::agent_inventory::AvailableAgent,
    usage: &AgentUsageMap,
    now: u64,
) -> AgentPickerRow {
    let description = compact(&agent.use_criteria);
    let (description, description_harness, provenance) = match &agent.source {
        crate::agent_inventory::AgentSource::Configured { .. } => {
            (nonempty(description, "Configured agent"), None, None)
        }
        crate::agent_inventory::AgentSource::NativeProfile { .. } => {
            let label = format!("{} profile", harness_label(agent.harness));
            (
                nonempty(description, "Native agent profile"),
                None,
                Some(AgentProvenance {
                    label,
                    harness: agent.harness,
                }),
            )
        }
        crate::agent_inventory::AgentSource::Generic => (
            format!("Generic {} agent", harness_label(agent.harness)),
            Some(agent.harness),
            None,
        ),
    };
    AgentPickerRow {
        name: agent.slug.clone(),
        description,
        description_harness,
        usage: usage_detail(usage_for(usage, agent), now),
        provenance,
    }
}

fn nonempty(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn usage_detail(usage: &usage::AgentUsage, now: u64) -> Option<String> {
    (usage.last_used != 0).then(|| {
        let uses = if usage.recent_uses == 1 {
            "use"
        } else {
            "uses"
        };
        format!(
            "{} {uses} / 30d · {}",
            usage.recent_uses,
            crate::util::relative_time(usage.last_used, now)
        )
    })
}

fn harness_label(harness: crate::session::Harness) -> &'static str {
    match harness {
        crate::session::Harness::ClaudeCode => "Claude",
        crate::session::Harness::Codex => "Codex",
        crate::session::Harness::Opencode => "OpenCode",
        crate::session::Harness::Grok => "Grok",
        crate::session::Harness::Unknown => "Unknown",
    }
}

fn compact(value: &str) -> String {
    value
        .replace("\\n", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
