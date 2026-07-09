use super::data::{HookTailOpts, HookTailSnapshot, RootPopup};
use super::loader::load_hook_tail_snapshot;
use super::render::{render_hook_tail, HookTailState};
use super::util::cycle_filter;
use anyhow::Result;
use crossterm::cursor as cursor_cmds;
use crossterm::{
    event::{self, KeyCode},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::{Duration, Instant};

struct TuiTerminal;

impl TuiTerminal {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, cursor_cmds::Hide)?;
        Ok(Self)
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), cursor_cmds::Show, LeaveAlternateScreen);
    }
}

pub(super) fn hook_tail(opts: HookTailOpts) -> Result<()> {
    let mut state = HookTailState {
        root_filters: opts.roots.into_iter().collect(),
        session_filter: opts.session,
        pane_limit: opts.panes.clamp(1, 24),
        focused: 0,
        focused_session: None,
        focus_mode: false,
        line_cursor: usize::MAX,
        detail_open: false,
        status: String::new(),
        popup: None,
    };

    let refresh = opts.refresh.max(Duration::from_millis(100));
    let mut snapshot = load_hook_tail_snapshot(&state.root_filters, &state.session_filter);
    let mut pane_order = Vec::new();
    stabilize_pane_order(&mut snapshot, &mut pane_order);

    let _terminal = TuiTerminal::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut next_refresh = Instant::now();
    let (snap_tx, snap_rx) = std::sync::mpsc::channel::<HookTailSnapshot>();
    let mut loading = false;

    loop {
        while let Ok(mut new) = snap_rx.try_recv() {
            stabilize_pane_order(&mut new, &mut pane_order);
            snapshot = new;
            loading = false;
        }

        // Re-anchor focused index to the same session after a snapshot re-sort.
        if let Some(ref sess) = state.focused_session {
            if let Some(idx) = snapshot.panes.iter().position(|p| &p.session == sess) {
                state.focused = idx;
            }
        }
        if state.focused >= snapshot.panes.len().max(1) {
            state.focused = snapshot.panes.len().saturating_sub(1);
            state.focused_session = snapshot.panes.get(state.focused).map(|p| p.session.clone());
        }

        terminal.draw(|f| render_hook_tail(f, &snapshot, &state))?;

        let wait = next_refresh
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));

        if event::poll(wait)? {
            if let event::Event::Key(key) = event::read()? {
                if state.detail_open {
                    // Any key closes the detail overlay
                    match key.code {
                        KeyCode::Char('q') => break,
                        _ => state.detail_open = false,
                    }
                } else if let Some(popup) = &mut state.popup {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('p') | KeyCode::Enter => {
                            state.popup = None;
                            next_refresh = Instant::now();
                        }
                        KeyCode::Up => {
                            if popup.cursor > 0 {
                                popup.cursor -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if popup.cursor + 1 < snapshot.roots.len() {
                                popup.cursor += 1;
                            }
                        }
                        KeyCode::Char(' ') => {
                            if let Some(root) = snapshot.roots.get(popup.cursor) {
                                if state.root_filters.contains(root) {
                                    state.root_filters.remove(root);
                                } else {
                                    state.root_filters.insert(root.clone());
                                }
                            }
                        }
                        KeyCode::Char('a') => {
                            state.root_filters.clear();
                        }
                        _ => {}
                    }
                } else if state.focus_mode {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Esc | KeyCode::Char('f') => {
                            state.focus_mode = false;
                        }
                        KeyCode::Enter => {
                            state.detail_open = true;
                        }
                        KeyCode::Up => {
                            let pane_len = snapshot
                                .panes
                                .get(state.focused)
                                .map(|p| p.lines.len())
                                .unwrap_or(0);
                            if state.line_cursor == usize::MAX {
                                state.line_cursor = pane_len.saturating_sub(2);
                            } else {
                                state.line_cursor = state.line_cursor.saturating_sub(1);
                            }
                        }
                        KeyCode::Down => {
                            let pane_len = snapshot
                                .panes
                                .get(state.focused)
                                .map(|p| p.lines.len())
                                .unwrap_or(0);
                            let last = pane_len.saturating_sub(1);
                            if state.line_cursor >= last {
                                state.line_cursor = usize::MAX; // snap to tail
                            } else {
                                state.line_cursor += 1;
                            }
                        }
                        KeyCode::Tab | KeyCode::Right => {
                            let n = snapshot.panes.len().min(state.pane_limit).max(1);
                            state.focused = (state.focused + 1) % n;
                            state.focused_session =
                                snapshot.panes.get(state.focused).map(|p| p.session.clone());
                            state.line_cursor = usize::MAX;
                        }
                        KeyCode::BackTab | KeyCode::Left => {
                            let n = snapshot.panes.len().min(state.pane_limit).max(1);
                            state.focused = if state.focused == 0 {
                                n - 1
                            } else {
                                state.focused - 1
                            };
                            state.focused_session =
                                snapshot.panes.get(state.focused).map(|p| p.session.clone());
                            state.line_cursor = usize::MAX;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            state.pane_limit = (state.pane_limit + 1).min(24);
                        }
                        KeyCode::Char('-') => {
                            state.pane_limit = state.pane_limit.saturating_sub(1).max(1);
                            state.focused = state.focused.min(state.pane_limit.saturating_sub(1));
                            state.focused_session =
                                snapshot.panes.get(state.focused).map(|p| p.session.clone());
                        }
                        KeyCode::Tab | KeyCode::Right => {
                            let n = snapshot.panes.len().min(state.pane_limit).max(1);
                            state.focused = (state.focused + 1) % n;
                            state.focused_session =
                                snapshot.panes.get(state.focused).map(|p| p.session.clone());
                        }
                        KeyCode::BackTab | KeyCode::Left => {
                            let n = snapshot.panes.len().min(state.pane_limit).max(1);
                            state.focused = if state.focused == 0 {
                                n - 1
                            } else {
                                state.focused - 1
                            };
                            state.focused_session =
                                snapshot.panes.get(state.focused).map(|p| p.session.clone());
                        }
                        KeyCode::Enter | KeyCode::Char('f') => {
                            state.focused_session =
                                snapshot.panes.get(state.focused).map(|p| p.session.clone());
                            state.focus_mode = true;
                            state.line_cursor = usize::MAX;
                        }
                        KeyCode::Char('a') => {
                            state.root_filters.clear();
                            state.session_filter = None;
                            state.status = "filters cleared".to_string();
                            next_refresh = Instant::now();
                        }
                        KeyCode::Char('p') => {
                            state.popup = Some(RootPopup { cursor: 0 });
                        }
                        KeyCode::Char('s') => {
                            state.session_filter =
                                cycle_filter(state.session_filter.as_deref(), &snapshot.sessions);
                            state.status = match &state.session_filter {
                                Some(s) => format!("session filter: {s}"),
                                None => "session filter cleared".to_string(),
                            };
                            next_refresh = Instant::now();
                        }
                        _ => {}
                    }
                }
            }
        }

        if !loading && Instant::now() >= next_refresh {
            next_refresh = Instant::now() + refresh;
            loading = true;
            let tx = snap_tx.clone();
            let filter_p = state.root_filters.clone();
            let filter_s = state.session_filter.clone();
            std::thread::spawn(move || {
                let snap = load_hook_tail_snapshot(&filter_p, &filter_s);
                let _ = tx.send(snap);
            });
        }
    }

    Ok(())
}

fn stabilize_pane_order(snapshot: &mut HookTailSnapshot, pane_order: &mut Vec<String>) {
    for pane in &snapshot.panes {
        if !pane_order.contains(&pane.session) {
            pane_order.push(pane.session.clone());
        }
    }

    snapshot.panes.sort_by_key(|pane| {
        pane_order
            .iter()
            .position(|session| session == &pane.session)
            .unwrap_or(usize::MAX)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::debug::data::SessionPane;

    fn pane(session: &str) -> SessionPane {
        SessionPane {
            session: session.to_string(),
            short: session.to_string(),
            ..SessionPane::default()
        }
    }

    #[test]
    fn pane_order_stays_fixed_when_snapshot_recency_order_changes() {
        let mut order = Vec::new();
        let mut first = HookTailSnapshot {
            panes: vec![pane("session-b"), pane("session-a")],
            ..HookTailSnapshot::default()
        };
        stabilize_pane_order(&mut first, &mut order);

        let mut next = HookTailSnapshot {
            panes: vec![pane("session-a"), pane("session-b"), pane("session-c")],
            ..HookTailSnapshot::default()
        };
        stabilize_pane_order(&mut next, &mut order);

        let sessions: Vec<_> = next
            .panes
            .iter()
            .map(|pane| pane.session.as_str())
            .collect();
        assert_eq!(sessions, vec!["session-b", "session-a", "session-c"]);
    }
}
