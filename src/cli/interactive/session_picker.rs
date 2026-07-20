mod data;
mod layout;
mod picker;

use self::data::SessionRow;
use crate::cli::agents::AgentRow;
use anyhow::{bail, Result};
use std::io::IsTerminal;

#[derive(Clone, Debug)]
struct SessionChoice {
    row: SessionRow,
}

#[derive(Clone, Debug)]
enum HomeChoice {
    Session(SessionChoice),
    Agent(AgentRow),
}

impl HomeChoice {
    pub(super) fn stable_id(&self) -> String {
        match self {
            Self::Session(choice) => format!("session:{}", choice.row.stable_id()),
            Self::Agent(row) => format!("agent:{}", row.slug),
        }
    }

    pub(super) fn fuzzy_score(&self, query: &str) -> Option<i64> {
        match self {
            Self::Session(choice) => choice.row.fuzzy_score(query),
            Self::Agent(row) => row.fuzzy_score(query),
        }
    }

    pub(super) fn is_session(&self) -> bool {
        matches!(self, Self::Session(_))
    }
}

pub(in crate::cli) async fn home() -> Result<()> {
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut focus = None;
    loop {
        let choices = load_choices().await?;
        if !interactive {
            print_summary(&choices);
            return Ok(());
        }
        let (_, terminal_rows) = crossterm::terminal::size().unwrap_or((100, 28));
        match picker::select(choices, terminal_rows, focus.as_deref()).await? {
            picker::PickerAction::Attach(choice) => {
                let Some(pty_id) = choice.row.pty_id else {
                    bail!("selected session no longer has an attachable endpoint");
                };
                return crate::pty::attach(&pty_id, &choice.row.handle);
            }
            picker::PickerAction::TakeOver(choice, interrupt_turn) => {
                return take_over(choice, interrupt_turn).await;
            }
            picker::PickerAction::Resume(choice) => {
                return crate::cli::launch_cli::attach_or_resume(&choice.row.npub)
                    .await
                    .and_then(|found| {
                        found.then_some(()).ok_or_else(|| {
                            anyhow::anyhow!("selected session disappeared before restart")
                        })
                    });
            }
            picker::PickerAction::Kill(choice) => kill(choice).await?,
            picker::PickerAction::Launch(row) => {
                return crate::cli::launch_cli::verbs::launch(
                    crate::cli::launch_cli::LaunchRequest {
                        agent: row.slug,
                        root: None,
                        channel: None,
                        session_name: None,
                        prompt: None,
                    },
                )
                .await;
            }
            picker::PickerAction::Edit(row) => {
                focus = Some(format!("agent:{}", row.slug));
                crate::cli::agents::edit_inventory_row(&row).await?;
            }
            picker::PickerAction::Delete(plan) => {
                focus = plan.first().map(|(row, _)| format!("agent:{}", row.slug));
                for (row, scope) in plan {
                    crate::cli::agents::delete_inventory_row(&row, scope).await?;
                }
            }
            picker::PickerAction::Cancel => return Ok(()),
        }
    }
}

async fn load_choices() -> Result<Vec<HomeChoice>> {
    let mut choices = data::fetch_sessions()
        .await?
        .into_iter()
        .map(|row| HomeChoice::Session(SessionChoice { row }))
        .collect::<Vec<_>>();
    choices.extend(
        crate::cli::agents::ordered_inventory()
            .await?
            .into_iter()
            .map(HomeChoice::Agent),
    );
    Ok(choices)
}

fn print_summary(choices: &[HomeChoice]) {
    println!("Sessions");
    let sessions = choices.iter().filter_map(|choice| match choice {
        HomeChoice::Session(choice) if choice.row.running => Some(&choice.row),
        _ => None,
    });
    let mut session_count = 0;
    for row in sessions {
        session_count += 1;
        let workspaces = row
            .workspaces
            .iter()
            .map(|workspace| workspace.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  @{}  {} · {}",
            row.handle,
            row.state,
            if workspaces.is_empty() {
                "no workspace"
            } else {
                &workspaces
            }
        );
    }
    if session_count == 0 {
        println!("  No live sessions");
    }

    println!("\nStart a session");
    let agents = choices
        .iter()
        .filter_map(|choice| match choice {
            HomeChoice::Agent(row) => Some(row),
            HomeChoice::Session(_) => None,
        })
        .collect::<Vec<_>>();
    let name_width = agents
        .iter()
        .map(|row| row.slug.chars().count())
        .max()
        .unwrap_or(0);
    for row in &agents {
        println!(
            "  {:name_width$}  {} · {}",
            row.slug,
            crate::cli::agents::harness_name(row.harness),
            row.summary(88)
        );
    }
    if agents.is_empty() {
        println!("  No launchable agents");
    }
}

async fn take_over(choice: SessionChoice, interrupt_turn: Option<u64>) -> Result<()> {
    let value = crate::cli::daemon_call_async(
        "session_pty_wrap",
        serde_json::json!({
            "session": choice.row.npub,
            "interrupt_working": interrupt_turn.is_some(),
            "turn_count": interrupt_turn.unwrap_or(0),
        }),
    )
    .await?;
    if value["wrapped"].as_bool().unwrap_or(false) {
        let pty_id = value["pty_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("takeover succeeded without a PTY endpoint"))?;
        return crate::pty::attach(pty_id, &choice.row.handle);
    }
    bail!(
        "could not take over @{}: {}",
        choice.row.handle,
        value["reason"].as_str().unwrap_or("takeover refused")
    )
}

async fn kill(choice: SessionChoice) -> Result<()> {
    let value = crate::cli::daemon_call_async(
        "session_kill",
        serde_json::json!({
            "session": choice.row.npub,
            "pty_id": choice.row.pty_id,
            "forget": true,
        }),
    )
    .await?;
    if value["killed"].as_bool().unwrap_or(false) {
        if !value["cleanup_confirmed"].as_bool().unwrap_or(true) {
            let failures = value["cleanup_failures"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join("; ");
            bail!(
                "killed @{}, but immediate fabric cleanup was not confirmed: {failures}",
                choice.row.handle
            );
        }
        return Ok(());
    }
    bail!(
        "could not kill @{}: {}",
        choice.row.handle,
        value["reason"].as_str().unwrap_or("kill failed")
    )
}
