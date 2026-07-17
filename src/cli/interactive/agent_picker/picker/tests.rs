use super::*;
use crate::cli::interactive::agent_picker::AgentProvenance;
use crate::session::Harness;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, layout::Rect, Terminal, TerminalOptions, Viewport};

fn row(name: &str) -> AgentPickerRow {
    AgentPickerRow {
        name: name.into(),
        description: format!("{name} description"),
        status: None,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn edit_delete_and_filter_keys_are_distinct() {
    let mut edit = PickerState::new(vec![row("one")]);
    assert_eq!(
        edit.handle_key(key(KeyCode::Char('e')), 10),
        Some(PickerAction::Edit(0))
    );

    let mut delete = PickerState::new(vec![row("one")]);
    assert_eq!(
        delete.handle_key(key(KeyCode::Char('d')), 10),
        Some(PickerAction::Delete(0))
    );

    let mut filter = PickerState::new(vec![row("editor"), row("writer")]);
    filter.handle_key(key(KeyCode::Char('/')), 10);
    filter.handle_key(key(KeyCode::Char('e')), 10);
    assert!(filter.filtering);
    assert_eq!(filter.query, "e");
    assert_eq!(filter.visible, vec![0, 1]);
}

#[test]
fn viewport_uses_terminal_height_up_to_forty_agent_rows() {
    assert_eq!(viewport_height(30), 30);
    assert_eq!(viewport_height(50), 42);
    assert_eq!(viewport_height(1), 1);
    assert_eq!(option_rows(42), 40);
}

#[test]
fn status_tracks_the_focused_harness() {
    let mut state = PickerState::new(vec![
        AgentPickerRow {
            name: "reviewer".into(),
            description: "Reviews".into(),
            status: Some(AgentProvenance {
                label: "Claude".into(),
                harness: Harness::ClaudeCode,
            }),
        },
        AgentPickerRow {
            name: "builder".into(),
            description: "Builds".into(),
            status: Some(AgentProvenance {
                label: "Codex".into(),
                harness: Harness::Codex,
            }),
        },
    ]);
    let backend = TestBackend::new(100, 5);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 100, 5)),
        },
    )
    .unwrap();

    let first = terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let first_status = first.buffer.content()[400..500]
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(first_status.contains("Claude"));
    assert_eq!(
        first.buffer.content()[400].fg,
        crate::console_style::harness_ratatui_color(Harness::ClaudeCode)
    );

    state.handle_key(key(KeyCode::Down), 3);
    let second = terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let second_status = second.buffer.content()[400..500]
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(second_status.contains("Codex"));
    assert_eq!(
        second.buffer.content()[400].fg,
        crate::console_style::harness_ratatui_color(Harness::Codex)
    );
}
