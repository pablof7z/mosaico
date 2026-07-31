use super::data;
use super::render;
use super::state::{Exit, PickerState};
use anyhow::{Context, Result};
use crossterm::{
    cursor::Show,
    event::{self, Event},
    execute, terminal,
};
use dialoguer::{theme::ColorfulTheme, Input};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal, TerminalOptions, Viewport};
use std::{io, time::Duration};

const CHROME_ROWS: u16 = 2;

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

pub(super) async fn run() -> Result<()> {
    let mut focus: Option<String> = None;
    loop {
        match run_session(focus.as_deref()).await? {
            SessionOutcome::Quit => return Ok(()),
            SessionOutcome::Continue { focus: next } => focus = next,
        }
    }
}

enum SessionOutcome {
    Quit,
    Continue { focus: Option<String> },
}

async fn run_session(initial_focus: Option<&str>) -> Result<SessionOutcome> {
    let forest = data::fetch_forest().await?;
    let (_, terminal_rows) = crossterm::terminal::size().unwrap_or((100, 28));
    let height = terminal_rows.max(1);
    let raw_mode = RawMode::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )
    .context("creating channel manager")?;
    terminal.hide_cursor()?;

    let mut state = PickerState::new(forest);
    if let Some(path) = initial_focus {
        state.focus_path(path);
    }
    let mut last_area = Rect::new(0, 0, 0, height);

    let outcome = loop {
        match interaction_step(&mut terminal, &mut state, &mut last_area).await? {
            None => continue,
            Some(Exit::Quit) => break SessionOutcome::Quit,
            Some(Exit::Edit { path, about }) => {
                cleanup_terminal(&mut terminal, last_area)?;
                drop(terminal);
                drop(raw_mode);
                match prompt_about(&path, &about) {
                    Ok(Some(new_about)) if new_about != about => {
                        if let Err(error) = data::edit_about(&path, &new_about).await {
                            eprintln!("edit failed: {error:#}");
                        }
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("edit cancelled: {error:#}"),
                }
                return Ok(SessionOutcome::Continue { focus: Some(path) });
            }
            Some(Exit::Delete { path }) => match data::delete_channel(&path).await {
                Ok(result) => {
                    let notified = result["notified_agents"]
                        .as_array()
                        .map(|a| a.len())
                        .unwrap_or(0);
                    state.set_notice(format!(
                        "deleted {path} · notified {notified} online agent(s)"
                    ));
                    match data::fetch_forest().await {
                        Ok(forest) => state.replace_forest(forest),
                        Err(error) => {
                            state.set_notice(format!("deleted {path}, refresh failed: {error:#}"))
                        }
                    }
                }
                Err(error) => state.set_notice(format!("delete failed: {error:#}")),
            },
        }
    };

    cleanup_terminal(&mut terminal, last_area)?;
    drop(terminal);
    drop(raw_mode);
    Ok(outcome)
}

async fn interaction_step(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut PickerState,
    last_area: &mut Rect,
) -> Result<Option<Exit>> {
    let lines = option_lines(last_area.height);
    state.ensure_visible(lines);
    *last_area = terminal
        .draw(|frame| render::draw(frame, state))
        .context("drawing channel manager")?
        .area;

    if !event::poll(Duration::from_millis(250)).context("polling channel manager")? {
        return Ok(None);
    }
    let Event::Key(key) = event::read().context("reading channel manager input")? else {
        return Ok(None);
    };
    if PickerState::wants_refresh(&key) {
        match data::fetch_forest().await {
            Ok(forest) => {
                state.replace_forest(forest);
                state.set_notice("refreshed");
            }
            Err(error) => state.set_notice(format!("refresh failed: {error:#}")),
        }
        return Ok(None);
    }
    Ok(state.handle_key(key, lines))
}

fn prompt_about(path: &str, current: &str) -> Result<Option<String>> {
    let theme = ColorfulTheme::default();
    let input: String = Input::with_theme(&theme)
        .with_prompt(format!("About for {path}"))
        .with_initial_text(current)
        .allow_empty(true)
        .interact_text()?;
    crate::channel_about::validate_channel_about(&input)?;
    Ok(Some(input))
}

fn cleanup_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    area: Rect,
) -> Result<()> {
    let clear = (area.width > 0).then(|| terminal.clear()).transpose();
    let position = terminal.set_cursor_position((0, area.y));
    let cursor = terminal.show_cursor();
    clear.context("clearing channel manager")?;
    position.context("restoring terminal cursor position")?;
    cursor.context("showing terminal cursor")?;
    Ok(())
}

fn option_lines(viewport_height: u16) -> usize {
    usize::from(viewport_height.saturating_sub(CHROME_ROWS))
}
