mod delete_flow;
mod render;
#[cfg(test)]
mod tests;

use super::{AgentPickerRow, DeleteScope};
use anyhow::{Context, Result};
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, terminal,
};
use delete_flow::PendingDelete;
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal, TerminalOptions, Viewport};
use std::collections::BTreeSet;
use std::io;

const MAX_VISIBLE_ROWS: u16 = 40;
const CHROME_ROWS: u16 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::cli) enum PickerAction {
    Launch(usize),
    Edit(usize),
    /// One `(row index, scope)` pair per agent to delete. A single-row
    /// delete with both a configured entry and a native profile lets the
    /// scope differ from `Both`; a multi-select delete always uses `Both`.
    Delete(Vec<(usize, DeleteScope)>),
    Cancel,
}

#[derive(Debug)]
struct PickerState {
    rows: Vec<AgentPickerRow>,
    visible: Vec<usize>,
    query: String,
    cursor: usize,
    offset: usize,
    selected: BTreeSet<usize>,
    pending_delete: Option<PendingDelete>,
}

impl PickerState {
    fn new(rows: Vec<AgentPickerRow>, initial_cursor: usize) -> Self {
        let visible: Vec<usize> = (0..rows.len()).collect();
        let cursor = initial_cursor.min(visible.len().saturating_sub(1));
        Self {
            rows,
            visible,
            query: String::new(),
            cursor,
            offset: 0,
            selected: BTreeSet::new(),
            pending_delete: None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, rows: usize) -> Option<PickerAction> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if self.pending_delete.is_some() {
            let pending = self.pending_delete.take().expect("checked above");
            return self.handle_pending_delete(pending, key);
        }
        match key.code {
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.refilter();
            }
            KeyCode::Esc => return Some(PickerAction::Cancel),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(PickerAction::Cancel);
            }
            KeyCode::Enter => return self.current().map(PickerAction::Launch),
            KeyCode::Char('e' | 'E') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.current().map(PickerAction::Edit);
            }
            KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(index) = self.current() {
                    if !self.selected.remove(&index) {
                        self.selected.insert(index);
                    }
                }
            }
            KeyCode::Char('d' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.begin_delete();
            }
            KeyCode::Up => self.move_up(1),
            KeyCode::Down => self.move_down(1),
            KeyCode::PageUp => self.move_up(rows.max(1)),
            KeyCode::PageDown => self.move_down(rows.max(1)),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.visible.len().saturating_sub(1),
            KeyCode::Backspace if !self.query.is_empty() => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(character);
                self.refilter();
            }
            _ => {}
        }
        self.ensure_visible(rows);
        None
    }

    fn current(&self) -> Option<usize> {
        self.visible.get(self.cursor).copied()
    }

    fn current_row(&self) -> Option<&AgentPickerRow> {
        self.current().map(|index| &self.rows[index])
    }

    fn move_up(&mut self, amount: usize) {
        if self.visible.is_empty() {
            return;
        }
        self.cursor = if amount == 1 && self.cursor == 0 {
            self.visible.len() - 1
        } else {
            self.cursor.saturating_sub(amount)
        };
    }

    fn move_down(&mut self, amount: usize) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        self.cursor = if amount == 1 && self.cursor == last {
            0
        } else {
            self.cursor.saturating_add(amount).min(last)
        };
    }

    fn refilter(&mut self) {
        let mut scored = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.fuzzy_score(&self.query).map(|score| (index, score)))
            .collect::<Vec<_>>();
        scored.sort_by(|(left_index, left_score), (right_index, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| left_index.cmp(right_index))
        });
        self.visible = scored.into_iter().map(|(index, _)| index).collect();
        self.cursor = 0;
        self.offset = 0;
    }

    fn ensure_visible(&mut self, rows: usize) {
        if rows == 0 || self.visible.is_empty() {
            self.offset = 0;
        } else if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + rows {
            self.offset = self.cursor + 1 - rows;
        }
    }

    fn window(&self, rows: usize) -> impl Iterator<Item = (usize, &AgentPickerRow)> {
        let end = (self.offset + rows).min(self.visible.len());
        self.visible[self.offset..end]
            .iter()
            .enumerate()
            .map(move |(relative, &row)| (self.offset + relative, &self.rows[row]))
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

pub(in crate::cli) fn select(
    rows: Vec<AgentPickerRow>,
    initial_cursor: usize,
) -> Result<PickerAction> {
    let (_, terminal_height) = terminal::size().unwrap_or((100, 28));
    let height = viewport_height(terminal_height);
    let _raw_mode = RawMode::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )
    .context("creating inline agent picker")?;
    terminal.hide_cursor()?;

    let mut state = PickerState::new(rows, initial_cursor);
    let mut last_area = Rect::new(0, 0, 0, height);
    let interaction = interaction_loop(&mut terminal, &mut state, &mut last_area);
    let cleanup = cleanup_terminal(&mut terminal, last_area);
    drop(terminal);
    cleanup?;
    interaction
}

fn interaction_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut PickerState,
    last_area: &mut Rect,
) -> Result<PickerAction> {
    loop {
        let rows = option_rows(last_area.height);
        state.ensure_visible(rows);
        *last_area = terminal
            .draw(|frame| render::draw(frame, state))
            .context("drawing agent picker")?
            .area;
        let Event::Key(key) = event::read().context("reading agent picker input")? else {
            continue;
        };
        if let Some(action) = state.handle_key(key, option_rows(last_area.height)) {
            return Ok(action);
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
    clear.context("clearing agent picker")?;
    position.context("restoring terminal cursor position")?;
    cursor.context("showing terminal cursor")?;
    Ok(())
}

fn option_rows(height: u16) -> usize {
    usize::from(height.saturating_sub(CHROME_ROWS))
}

fn viewport_height(terminal_height: u16) -> u16 {
    terminal_height.clamp(1, MAX_VISIBLE_ROWS + CHROME_ROWS)
}
