mod data;
mod layout;

use self::data::SessionRow;
use self::layout::SessionLayout;
use super::prompt::{install_theme, prompted};
use anyhow::{bail, Result};
use inquire::{list_option::ListOption, Confirm, MultiSelect};
use std::fmt::{self, Display};
use std::io::IsTerminal;

const HELP: &str = "type filter · ↑↓ move · space toggle · → all · ← none · enter · esc";
const OPTION_CHROME_WIDTH: usize = 7;

#[derive(Clone, Debug)]
struct SessionChoice {
    label: String,
    row: SessionRow,
}

impl SessionChoice {
    fn new(row: SessionRow, now: u64, layout: &SessionLayout) -> Self {
        let label = layout.row(&row, now);
        Self { label, row }
    }
}

impl Display for SessionChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

pub(in crate::cli) async fn session_list() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("tenex-edge mgmt session list is interactive — run it in a terminal");
    }
    install_theme();

    let rows = data::fetch_sessions().await?;
    if rows.is_empty() {
        println!("No local sessions.");
        return Ok(());
    }

    let now = crate::util::now_secs();
    let (columns, terminal_rows) = crossterm::terminal::size().unwrap_or((100, 28));
    let option_width = usize::from(columns)
        .saturating_sub(OPTION_CHROME_WIDTH)
        .max(1);
    let page_size = layout::picker_page_size(rows.len(), usize::from(terminal_rows));
    let layout = SessionLayout::new(&rows, option_width);
    let prompt = format!(
        "Select sessions to kill\n      {}\n  Filter:",
        layout.header()
    );
    let choices = rows
        .into_iter()
        .map(|row| SessionChoice::new(row, now, &layout))
        .collect::<Vec<_>>();
    let Some(selected) = prompted(
        MultiSelect::new(&prompt, choices)
            .with_page_size(page_size)
            .with_help_message(HELP)
            .with_scorer(&score_choice)
            .with_formatter(&format_selection)
            .prompt(),
    )?
    else {
        return Ok(());
    };

    if selected.is_empty() {
        println!("No sessions selected.");
        return Ok(());
    }

    let question = kill_confirmation(&selected);
    let Some(confirmed) = prompted(Confirm::new(&question).with_default(false).prompt())? else {
        return Ok(());
    };
    if !confirmed {
        println!("No sessions killed.");
        return Ok(());
    }

    kill_selected(selected).await
}

fn score_choice(input: &str, choice: &SessionChoice, _: &str, _: usize) -> Option<i64> {
    choice.row.fuzzy_score(input)
}

fn format_selection(selected: &[ListOption<&SessionChoice>]) -> String {
    format!("{} session(s) selected", selected.len())
}

fn kill_confirmation(selected: &[SessionChoice]) -> String {
    let handles = selected
        .iter()
        .take(3)
        .map(|choice| format!("@{}", choice.row.handle))
        .collect::<Vec<_>>()
        .join(", ");
    let remaining = selected.len().saturating_sub(3);
    if remaining == 0 {
        format!("Kill {} session(s): {handles}?", selected.len())
    } else {
        format!(
            "Kill {} session(s): {handles}, and {remaining} more?",
            selected.len()
        )
    }
}

async fn kill_selected(selected: Vec<SessionChoice>) -> Result<()> {
    let mut killed = 0usize;
    let mut failures = Vec::new();
    for choice in selected {
        let result = crate::cli::daemon_call_async(
            "session_kill",
            serde_json::json!({ "session": choice.row.session_id }),
        )
        .await;
        match result {
            Ok(value) if value["killed"].as_bool().unwrap_or(false) => killed += 1,
            Ok(value) => failures.push(format!(
                "@{}: {}",
                choice.row.handle,
                value["reason"].as_str().unwrap_or("kill failed")
            )),
            Err(error) => failures.push(format!("@{}: {error:#}", choice.row.handle)),
        }
    }

    if failures.is_empty() {
        println!("Killed {killed} session(s).");
        Ok(())
    } else {
        bail!(
            "killed {killed} session(s); failed: {}",
            failures.join("; ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(handle: &str) -> SessionChoice {
        let row = SessionRow {
            handle: handle.to_string(),
            ..SessionRow::default()
        };
        let layout = SessionLayout::new(std::slice::from_ref(&row), 80);
        SessionChoice::new(row, 100, &layout)
    }

    #[test]
    fn fuzzy_search_uses_hidden_projection_fields_and_prefers_handles() {
        let cwd_row = SessionRow {
            handle: "opal".into(),
            workspace: "tenex-edge".into(),
            cwd: Some("/repo/edge".into()),
            ..SessionRow::default()
        };
        let handle_row = SessionRow {
            handle: "delta-codex".into(),
            activity: "ordinary work".into(),
            ..SessionRow::default()
        };
        let incidental_row = SessionRow {
            handle: "other-codex".into(),
            activity: "reviewing delta output".into(),
            ..SessionRow::default()
        };
        let rows = [cwd_row.clone(), handle_row.clone(), incidental_row.clone()];
        let layout = SessionLayout::new(&rows, 80);
        let cwd_choice = SessionChoice::new(cwd_row, 100, &layout);
        let handle_choice = SessionChoice::new(handle_row, 100, &layout);
        let incidental_choice = SessionChoice::new(incidental_row, 100, &layout);

        assert!(score_choice("rpedge", &cwd_choice, "", 0).is_some());
        assert!(
            score_choice("delta", &handle_choice, "", 0)
                > score_choice("delta", &incidental_choice, "", 0)
        );
    }

    #[test]
    fn confirmation_names_small_and_large_selections() {
        assert_eq!(
            kill_confirmation(&[choice("one"), choice("two")]),
            "Kill 2 session(s): @one, @two?"
        );
        assert_eq!(
            kill_confirmation(&[
                choice("one"),
                choice("two"),
                choice("three"),
                choice("four"),
            ]),
            "Kill 4 session(s): @one, @two, @three, and 1 more?"
        );
    }
}
