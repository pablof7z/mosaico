use super::*;
use crate::cli::interactive::agent_picker::AgentProvenance;
use crate::session::Harness;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, layout::Rect, Terminal, TerminalOptions, Viewport};
use std::collections::BTreeSet;

fn row(name: &str) -> AgentPickerRow {
    AgentPickerRow {
        name: name.into(),
        description: format!("{name} description"),
        status: None,
        has_configured: false,
        has_native_profile: false,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn edit_delete_and_filter_keys_are_distinct() {
    let mut edit = PickerState::new(vec![row("one")], 0);
    assert_eq!(
        edit.handle_key(key(KeyCode::Char('e')), 10),
        Some(PickerAction::Edit(0))
    );

    let mut delete = PickerState::new(
        vec![AgentPickerRow {
            has_configured: true,
            ..row("one")
        }],
        0,
    );
    assert_eq!(delete.handle_key(key(KeyCode::Char('d')), 10), None);
    assert_eq!(
        delete.handle_key(key(KeyCode::Char('y')), 10),
        Some(PickerAction::Delete(vec![(0, DeleteScope::Agent)]))
    );

    let mut filter = PickerState::new(vec![row("editor"), row("writer")], 0);
    filter.handle_key(key(KeyCode::Char('/')), 10);
    filter.handle_key(key(KeyCode::Char('e')), 10);
    assert!(filter.filtering);
    assert_eq!(filter.query, "e");
    assert_eq!(filter.visible, vec![0, 1]);
}

#[test]
fn pending_delete_confirm_only_accepts_y_or_d_and_esc_cancels() {
    let mut state = PickerState::new(
        vec![AgentPickerRow {
            has_configured: true,
            ..row("one")
        }],
        0,
    );
    state.handle_key(key(KeyCode::Char('d')), 10);
    assert!(state.pending_delete.is_some());

    // An unrelated key neither confirms nor dismisses the prompt.
    assert_eq!(state.handle_key(key(KeyCode::Char('x')), 10), None);
    assert!(state.pending_delete.is_some());

    assert_eq!(state.handle_key(key(KeyCode::Esc), 10), None);
    assert!(state.pending_delete.is_none());

    state.handle_key(key(KeyCode::Char('d')), 10);
    assert_eq!(
        state.handle_key(key(KeyCode::Char('d')), 10),
        Some(PickerAction::Delete(vec![(0, DeleteScope::Agent)]))
    );
}

#[test]
fn space_toggles_multi_select_and_bulk_deletes_selected_rows() {
    let mut state = PickerState::new(
        vec![
            AgentPickerRow {
                has_configured: true,
                ..row("one")
            },
            AgentPickerRow {
                has_native_profile: true,
                ..row("two")
            },
            row("three"), // left unselected; generic, nothing to delete
        ],
        0,
    );

    state.handle_key(key(KeyCode::Char(' ')), 10);
    state.move_down(1);
    state.handle_key(key(KeyCode::Char(' ')), 10);
    assert_eq!(state.selected, BTreeSet::from([0usize, 1]));

    state.handle_key(key(KeyCode::Char('d')), 10);
    assert_eq!(
        state.handle_key(key(KeyCode::Char('y')), 10),
        Some(PickerAction::Delete(vec![
            (0, DeleteScope::Both),
            (1, DeleteScope::Both)
        ]))
    );
    // The selection is cleared once the delete is dispatched.
    assert!(state.selected.is_empty());
}

#[test]
fn initial_cursor_is_honored_and_clamped() {
    let state = PickerState::new(vec![row("one"), row("two"), row("three")], 2);
    assert_eq!(state.cursor, 2);

    let clamped = PickerState::new(vec![row("one")], 5);
    assert_eq!(clamped.cursor, 0);
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
    let mut state = PickerState::new(
        vec![
            AgentPickerRow {
                name: "reviewer".into(),
                description: "Reviews".into(),
                status: Some(AgentProvenance {
                    label: "Claude".into(),
                    harness: Harness::ClaudeCode,
                }),
                has_configured: false,
                has_native_profile: false,
            },
            AgentPickerRow {
                name: "builder".into(),
                description: "Builds".into(),
                status: Some(AgentProvenance {
                    label: "Codex".into(),
                    harness: Harness::Codex,
                }),
                has_configured: false,
                has_native_profile: false,
            },
        ],
        0,
    );
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
