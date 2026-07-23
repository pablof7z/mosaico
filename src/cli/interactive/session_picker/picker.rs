mod confirmation;
mod delete;
mod project;
mod range;
mod render;
mod state;
#[cfg(test)]
mod tests;

use super::{HomeChoice, SessionChoice};
use crate::cli::interactive::agent_picker::DeleteScope;
use anyhow::{Context, Result};
use crossterm::{
    cursor::Show,
    event::{self, Event},
    execute, terminal,
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal, TerminalOptions, Viewport};
use std::{io, time::Duration};

use self::state::PickerState;

const CHROME_ROWS: u16 = 2;
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub(super) enum PickerAction {
    Attach(SessionChoice),
    Resume(SessionChoice),
    TakeOver(SessionChoice, Option<u64>),
    Kill(SessionChoice),
    Launch(crate::cli::agents::AgentRow),
    Edit(crate::cli::agents::AgentRow),
    Delete(Vec<(crate::cli::agents::AgentRow, DeleteScope)>),
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PickerExit {
    Attach(usize),
    Resume(usize),
    TakeOver(usize, Option<u64>),
    Kill(usize),
    Launch(usize),
    Edit(usize),
    Delete(Vec<(usize, DeleteScope)>),
    Cancel,
}

struct RawMode;

impl RawMode {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("enabling raw terminal mode")?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show);
    }
}

pub(super) async fn select(
    choices: Vec<HomeChoice>,
    terminal_height: u16,
    initial_focus: Option<&str>,
    initial_project_filter: Option<&str>,
) -> Result<PickerAction> {
    let height = viewport_height(terminal_height);
    let _raw_mode = RawMode::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )
    .context("creating inline operator home")?;
    terminal.hide_cursor()?;

    let mut state = PickerState::new(choices, initial_focus)
        .with_project_filter(initial_project_filter.map(str::to_owned));
    let mut last_area = Rect::new(0, 0, 0, height);
    let interaction = interaction_loop(&mut terminal, &mut state, &mut last_area).await;
    let cleanup = cleanup_terminal(&mut terminal, last_area);
    drop(terminal);
    cleanup?;

    Ok(match interaction? {
        PickerExit::Attach(index) => PickerAction::Attach(take_session(&mut state, index)),
        PickerExit::Resume(index) => PickerAction::Resume(take_session(&mut state, index)),
        PickerExit::TakeOver(index, interrupt_turn) => {
            PickerAction::TakeOver(take_session(&mut state, index), interrupt_turn)
        }
        PickerExit::Kill(index) => PickerAction::Kill(take_session(&mut state, index)),
        PickerExit::Launch(index) => PickerAction::Launch(take_agent(&mut state, index)),
        PickerExit::Edit(index) => PickerAction::Edit(take_agent(&mut state, index)),
        PickerExit::Delete(plan) => PickerAction::Delete(
            plan.into_iter()
                .map(|(index, scope)| (state.agent(index).clone(), scope))
                .collect(),
        ),
        PickerExit::Cancel => PickerAction::Cancel,
    })
}

fn take_session(state: &mut PickerState, index: usize) -> SessionChoice {
    match state.choices.swap_remove(index) {
        HomeChoice::Session(choice) => choice,
        HomeChoice::Agent(_) => unreachable!("session action targeted an agent"),
    }
}

fn take_agent(state: &mut PickerState, index: usize) -> crate::cli::agents::AgentRow {
    match state.choices.swap_remove(index) {
        HomeChoice::Agent(row) => row,
        HomeChoice::Session(_) => unreachable!("agent action targeted a session"),
    }
}

async fn interaction_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut PickerState,
    last_area: &mut Rect,
) -> Result<PickerExit> {
    let mut next_refresh = std::time::Instant::now() + REFRESH_INTERVAL;
    loop {
        let lines = option_lines(last_area.height);
        state.ensure_visible(lines);
        *last_area = terminal
            .draw(|frame| render::draw(frame, state))
            .context("drawing operator home")?
            .area;

        let wait = next_refresh
            .saturating_duration_since(std::time::Instant::now())
            .min(Duration::from_millis(250));
        if event::poll(wait).context("polling operator home input")? {
            if let Event::Key(key) = event::read().context("reading operator home input")? {
                if let Some(exit) = state.handle_key(key, option_lines(last_area.height)) {
                    return Ok(exit);
                }
            }
        }

        if std::time::Instant::now() >= next_refresh && state.can_refresh() {
            match super::data::fetch_sessions().await {
                Ok(rows) => {
                    if state
                        .notice
                        .as_deref()
                        .is_some_and(|notice| notice.starts_with("Live refresh failed:"))
                    {
                        state.notice = None;
                    }
                    state.replace_sessions(
                        rows.into_iter().map(|row| SessionChoice { row }).collect(),
                    );
                }
                Err(error) => state.notice = Some(format!("Live refresh failed: {error:#}")),
            }
            next_refresh = std::time::Instant::now() + REFRESH_INTERVAL;
        }
    }
}

fn cleanup_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    area: Rect,
) -> Result<()> {
    let clear = (area.width > 0).then(|| terminal.clear()).transpose();
    let position = terminal.set_cursor_position((0, area.y));
    let cursor = terminal.show_cursor();
    clear.context("clearing operator home")?;
    position.context("restoring terminal cursor position")?;
    cursor.context("showing terminal cursor")?;
    Ok(())
}

fn option_lines(viewport_height: u16) -> usize {
    usize::from(viewport_height.saturating_sub(CHROME_ROWS))
}

fn viewport_height(terminal_height: u16) -> u16 {
    terminal_height.max(1)
}
