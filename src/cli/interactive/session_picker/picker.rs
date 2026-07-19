mod confirmation;
mod project;
mod render;
mod state;
#[cfg(test)]
mod tests;

use super::SessionChoice;
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
const OPTION_HEIGHT: u16 = 2;
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub(super) enum PickerAction {
    Attach(SessionChoice),
    Resume(SessionChoice),
    TakeOver(SessionChoice, Option<u64>),
    Kill(SessionChoice),
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerExit {
    Attach(usize),
    Resume(usize),
    TakeOver(usize, Option<u64>),
    Kill(usize),
    Cancel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SessionScope {
    #[default]
    Live,
    All,
}

impl SessionScope {
    fn toggle(&mut self) {
        *self = match self {
            Self::Live => Self::All,
            Self::All => Self::Live,
        };
    }

    fn label(self) -> &'static str {
        match self {
            Self::Live => "Live",
            Self::All => "All",
        }
    }
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
    choices: Vec<SessionChoice>,
    terminal_height: u16,
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
    .context("creating inline session picker")?;
    terminal.hide_cursor()?;

    let mut state = PickerState::new(choices);
    let mut last_area = Rect::new(0, 0, 0, height);
    let interaction = interaction_loop(&mut terminal, &mut state, &mut last_area).await;
    let cleanup = cleanup_terminal(&mut terminal, last_area);
    drop(terminal);
    cleanup?;

    Ok(match interaction? {
        PickerExit::Attach(index) => PickerAction::Attach(state.choices.swap_remove(index)),
        PickerExit::Resume(index) => PickerAction::Resume(state.choices.swap_remove(index)),
        PickerExit::TakeOver(index, interrupt_turn) => {
            PickerAction::TakeOver(state.choices.swap_remove(index), interrupt_turn)
        }
        PickerExit::Kill(index) => PickerAction::Kill(state.choices.swap_remove(index)),
        PickerExit::Cancel => PickerAction::Cancel,
    })
}

async fn interaction_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut PickerState,
    last_area: &mut Rect,
) -> Result<PickerExit> {
    let mut next_refresh = std::time::Instant::now() + REFRESH_INTERVAL;
    loop {
        let rows = option_rows(last_area.height);
        state.ensure_visible(rows);
        *last_area = terminal
            .draw(|frame| render::draw(frame, state))
            .context("drawing session picker")?
            .area;

        let wait = next_refresh
            .saturating_duration_since(std::time::Instant::now())
            .min(Duration::from_millis(250));
        if event::poll(wait).context("polling session picker input")? {
            if let Event::Key(key) = event::read().context("reading session picker input")? {
                if let Some(exit) = state.handle_key(key, option_rows(last_area.height)) {
                    return Ok(exit);
                }
            }
        }

        if std::time::Instant::now() >= next_refresh {
            match super::data::fetch_sessions().await {
                Ok(rows) => {
                    if state
                        .notice
                        .as_deref()
                        .is_some_and(|notice| notice.starts_with("Live refresh failed:"))
                    {
                        state.notice = None;
                    }
                    state.replace_choices(
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
    clear.context("clearing session picker")?;
    position.context("restoring terminal cursor position")?;
    cursor.context("showing terminal cursor")?;
    Ok(())
}

fn option_rows(viewport_height: u16) -> usize {
    usize::from(viewport_height.saturating_sub(CHROME_ROWS) / OPTION_HEIGHT)
}

fn viewport_height(terminal_height: u16) -> u16 {
    terminal_height.max(1)
}
