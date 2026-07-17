use super::data::{AgentKind, AgentRow};
use anyhow::{bail, Result};
use dialoguer::{theme::ColorfulTheme, Confirm, Select};
use std::io::IsTerminal as _;

#[derive(Clone, Copy)]
enum DeleteTarget {
    Agent,
    Profile,
    Both,
}

pub(super) async fn delete(row: &AgentRow) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!("agent deletion is interactive — run it in a terminal");
    }
    let target = match (
        row.kind == AgentKind::Configured,
        row.native_profile.is_some(),
    ) {
        (true, true) => {
            let choices = [
                "Delete agent configuration",
                "Delete native agent profile",
                "Delete both",
            ];
            match Select::with_theme(&ColorfulTheme::default())
                .with_prompt(format!("What should be deleted for {}?", row.slug))
                .items(&choices)
                .default(0)
                .interact()?
            {
                0 => DeleteTarget::Agent,
                1 => DeleteTarget::Profile,
                _ => DeleteTarget::Both,
            }
        }
        (true, false) => DeleteTarget::Agent,
        (false, true) => DeleteTarget::Profile,
        (false, false) => {
            println!("{} is a generic agent and has no file to delete", row.slug);
            return Ok(());
        }
    };
    let profile = row.native_profile.as_ref();
    let detail = match target {
        DeleteTarget::Agent => format!("agent configuration for {}", row.slug),
        DeleteTarget::Profile => format!("native profile {}", profile.unwrap().path.display()),
        DeleteTarget::Both => format!(
            "agent configuration and native profile {}",
            profile.unwrap().path.display()
        ),
    };
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Permanently delete {detail}?"))
        .default(false)
        .interact()?
    {
        return Ok(());
    }
    if matches!(target, DeleteTarget::Agent | DeleteTarget::Both)
        && crate::identity::remove_local_agent(&crate::config::mosaico_home(), &row.slug)?
    {
        println!("Deleted agent configuration {}", row.slug);
    }
    if matches!(target, DeleteTarget::Profile | DeleteTarget::Both)
        && crate::agent_catalog::remove_native_profile(profile.unwrap())?
    {
        println!("Deleted native profile {}", profile.unwrap().path.display());
    }
    super::schedule_roster_refresh(Some(&row.slug)).await;
    Ok(())
}
