use super::daemon_call_async;
use crate::identity::LocalAgent;
use crate::util::pubkey_short;
use anyhow::{bail, Result};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event as TermEvent, KeyCode, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use owo_colors::OwoColorize;
use std::collections::HashSet;
use std::io::{self, IsTerminal as _, Write as _};

struct AgentChoice {
    slug: String,
    pubkey: String,
    command: Option<Vec<String>>,
    selected: bool,
    initial: bool,
}

pub(super) async fn edit_membership(project: String) -> Result<()> {
    let edge_home = crate::config::edge_home();
    let local_agents = crate::identity::list_local_agent_details(&edge_home);
    if local_agents.is_empty() {
        println!("No local agents in {}", edge_home.join("agents").display());
        println!("Add one with: tenex-edge agent add <slug> [-- <command>]");
        return Ok(());
    }

    let current = current_project_members(&project).await?;
    let mut choices = local_agents
        .into_iter()
        .map(|agent| choice_from_agent(agent, &current))
        .collect::<Vec<_>>();

    if !run_selector(&project, &mut choices)? {
        println!("cancelled");
        return Ok(());
    }

    apply_membership_changes(&project, &choices).await
}

async fn current_project_members(project: &str) -> Result<HashSet<String>> {
    let v = daemon_call_async("project_members", serde_json::json!({ "project": project })).await?;
    let members = v["members"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    Ok(members
        .iter()
        .filter_map(|m| m["pubkey"].as_str())
        .map(str::to_string)
        .collect())
}

fn choice_from_agent(agent: LocalAgent, current: &HashSet<String>) -> AgentChoice {
    let selected = current.contains(&agent.pubkey);
    AgentChoice {
        slug: agent.slug,
        pubkey: agent.pubkey,
        command: agent.command,
        selected,
        initial: selected,
    }
}

fn run_selector(project: &str, choices: &mut [AgentChoice]) -> Result<bool> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("project add without PUBKEY requires an interactive terminal");
    }

    let _guard = TerminalGuard::enter()?;
    let mut active = 0usize;
    let mut offset = 0usize;

    loop {
        render(project, choices, active, &mut offset)?;
        match event::read()? {
            TermEvent::Key(key) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(false);
                }
                KeyCode::Esc | KeyCode::Char('q') => return Ok(false),
                KeyCode::Enter => return Ok(true),
                KeyCode::Up | KeyCode::Char('k') => {
                    active = active.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if active + 1 < choices.len() {
                        active += 1;
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(choice) = choices.get_mut(active) {
                        choice.selected = !choice.selected;
                    }
                }
                _ => {}
            },
            TermEvent::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn render(project: &str, choices: &[AgentChoice], active: usize, offset: &mut usize) -> Result<()> {
    let (_cols, rows) = terminal::size()?;
    let visible_rows = usize::from(rows.saturating_sub(6)).max(1);
    if active < *offset {
        *offset = active;
    } else if active >= *offset + visible_rows {
        *offset = active + 1 - visible_rows;
    }

    let mut out = io::stdout();
    execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;
    writeln!(out, "Project agents: {}", project.bold())?;
    writeln!(out, "Use up/down to move, space to toggle, enter to apply.")?;
    writeln!(out)?;

    for (idx, choice) in choices.iter().enumerate().skip(*offset).take(visible_rows) {
        let cursor = if idx == active { ">" } else { " " };
        let mark = if choice.selected { "[x]" } else { "[ ]" };
        let changed = if choice.selected != choice.initial {
            "*"
        } else {
            " "
        };
        let cmd = choice
            .command
            .as_ref()
            .map(|c| c.join(" "))
            .unwrap_or_else(|| "(default harness)".to_string());
        if idx == active {
            writeln!(
                out,
                "{} {}{} {}  {}  {}",
                cursor.bold(),
                mark.bold(),
                changed.yellow(),
                choice.slug.bold(),
                pubkey_short(&choice.pubkey).cyan(),
                cmd.dimmed()
            )?;
        } else {
            writeln!(
                out,
                "{cursor} {mark}{changed} {}  {}  {}",
                choice.slug,
                pubkey_short(&choice.pubkey).cyan(),
                cmd.dimmed()
            )?;
        }
    }

    if *offset > 0 || *offset + visible_rows < choices.len() {
        writeln!(out)?;
        writeln!(
            out,
            "{}",
            format!(
                "showing {}-{} of {}",
                *offset + 1,
                (*offset + visible_rows).min(choices.len()),
                choices.len()
            )
            .dimmed()
        )?;
    }
    out.flush()?;
    Ok(())
}

async fn apply_membership_changes(project: &str, choices: &[AgentChoice]) -> Result<()> {
    let mut changed = 0usize;
    let mut failed = 0usize;

    for choice in choices.iter().filter(|c| c.selected != c.initial) {
        changed += 1;
        let method = if choice.selected {
            "project_add"
        } else {
            "project_remove"
        };
        let verb = if choice.selected { "added" } else { "removed" };
        match daemon_call_async(
            method,
            serde_json::json!({ "project": project, "pubkey": choice.pubkey }),
        )
        .await
        {
            Ok(v) => {
                let resolved = v["pubkey"].as_str().unwrap_or(&choice.pubkey);
                println!(
                    "{} {} {} {}",
                    verb,
                    choice.slug.bold(),
                    pubkey_short(resolved).cyan(),
                    project.bold()
                );
            }
            Err(e) => {
                failed += 1;
                eprintln!(
                    "failed to {verb} {} {}: {e}",
                    choice.slug.bold(),
                    project.bold()
                );
            }
        }
    }

    if changed == 0 {
        println!("No membership changes for {}", project.bold());
    }
    if failed > 0 {
        bail!("{failed} membership change(s) failed");
    }
    Ok(())
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}
