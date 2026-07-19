use super::*;
use crate::cli::interactive::session_picker::data::SessionRow;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, layout::Rect, Terminal, TerminalOptions, Viewport};

fn choice(handle: &str, activity: &str, attachable: bool) -> SessionChoice {
    SessionChoice {
        row: SessionRow {
            handle: handle.into(),
            activity: activity.into(),
            pty_id: attachable.then(|| format!("pty-{handle}")),
            endpoint_live: attachable,
            endpoint_attachable: attachable,
            running: true,
            transport: if attachable {
                "pty".into()
            } else {
                "harness".into()
            },
            ..SessionRow::default()
        },
    }
}

fn takeover_choice(handle: &str, turn_open: bool) -> SessionChoice {
    SessionChoice {
        row: SessionRow {
            handle: handle.into(),
            transport: "process".into(),
            takeover_available: true,
            running: true,
            turn_open,
            turn_count: 7,
            ..SessionRow::default()
        },
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn picker_uses_the_terminal_height_and_counts_two_line_rows() {
    assert_eq!(viewport_height(50), 50);
    assert_eq!(viewport_height(1), 1);
    assert_eq!(option_rows(12), 5);
}

#[test]
fn enter_attaches_only_live_terminal_sessions() {
    let mut attachable = PickerState::new(vec![choice("opal", "", true)]);
    assert_eq!(
        attachable.handle_key(key(KeyCode::Enter), 10),
        Some(PickerExit::Attach(0))
    );

    let mut headless = PickerState::new(vec![choice("opal", "", false)]);
    assert_eq!(headless.handle_key(key(KeyCode::Enter), 10), None);
    assert!(headless.notice.as_deref().unwrap().contains("no live"));
}

#[test]
fn enter_on_acp_session_reports_no_harness_terminal() {
    let row = SessionRow {
        handle: "delta-claude".into(),
        transport: "acp".into(),
        running: true,
        ..SessionRow::default()
    };
    let mut state = PickerState::new(vec![SessionChoice { row }]);
    assert_eq!(state.handle_key(key(KeyCode::Enter), 10), None);
    let notice = state.notice.as_deref().unwrap();
    assert!(
        notice.contains("ACP"),
        "notice should mention ACP: {notice}"
    );
    assert!(
        notice.contains("without a harness") || notice.contains("no harness"),
        "notice should mention harness: {notice}"
    );
}

#[test]
fn takeover_always_requires_confirmation() {
    let mut state = PickerState::new(vec![takeover_choice("echo-codex", false)]);

    assert_eq!(state.handle_key(key(KeyCode::Enter), 10), None);
    assert!(state
        .confirmation_text()
        .unwrap()
        .contains("Kill @echo-codex"));
    assert_eq!(
        state.handle_key(key(KeyCode::Char('y')), 10),
        Some(PickerExit::TakeOver(0, None))
    );
}

#[test]
fn open_turn_requires_a_second_confirmation() {
    let mut state = PickerState::new(vec![takeover_choice("echo-codex", true)]);

    state.handle_key(key(KeyCode::Enter), 10);
    assert_eq!(state.handle_key(key(KeyCode::Char('y')), 10), None);
    let prompt = state.confirmation_text().unwrap();
    assert!(prompt.contains("No end-of-turn hook"), "{prompt}");
    assert_eq!(
        state.handle_key(key(KeyCode::Char('y')), 10),
        Some(PickerExit::TakeOver(0, Some(7)))
    );
}

#[test]
fn declining_takeover_returns_to_the_picker() {
    let mut state = PickerState::new(vec![takeover_choice("echo-codex", false)]);

    state.handle_key(key(KeyCode::Enter), 10);
    assert_eq!(state.handle_key(key(KeyCode::Char('n')), 10), None);
    assert!(state.confirmation.is_none());
    assert_eq!(state.handle_key(key(KeyCode::Down), 10), None);
}

#[test]
fn shift_k_kills_the_highlighted_session_without_selection_state() {
    let mut state = PickerState::new(vec![choice("one", "", true), choice("two", "", false)]);
    state.handle_key(key(KeyCode::Down), 10);
    let shift_k = KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT);
    assert_eq!(state.handle_key(shift_k, 10), Some(PickerExit::Kill(1)));
}

#[test]
fn search_reaches_exited_sessions_while_live_scope_is_selected() {
    let mut exited = choice("juno-codex", "finished earlier", false);
    exited.row.running = false;
    exited.row.state = crate::session_state::SessionState::Offline;
    exited.row.resumable = true;
    let mut state = PickerState::new(vec![choice("opal-codex", "working", true), exited]);

    assert_eq!(state.visible, vec![0]);
    for character in "juno".chars() {
        state.handle_key(key(KeyCode::Char(character)), 10);
    }
    assert_eq!(state.visible, vec![1]);
    assert_eq!(
        state.handle_key(key(KeyCode::Enter), 10),
        Some(PickerExit::Resume(1))
    );
}

#[test]
fn tab_expands_the_empty_query_from_live_sessions_to_all_history() {
    let mut exited = choice("juno-codex", "finished earlier", false);
    exited.row.running = false;
    exited.row.state = crate::session_state::SessionState::Offline;
    let mut state = PickerState::new(vec![choice("opal-codex", "working", true), exited]);

    state.handle_key(key(KeyCode::Tab), 10);

    assert_eq!(state.scope, SessionScope::All);
    assert_eq!(state.visible, vec![0, 1]);
}

#[test]
fn project_picker_applies_an_independent_workspace_filter() {
    let mut mosaico = choice("juno-codex", "mosaico work", false);
    mosaico.row.workspaces = vec![
        crate::cli::interactive::session_picker::data::WorkspaceGroup {
            id: "mosaico-id".into(),
            name: "mosaico".into(),
            path: Some("/repo/mosaico".into()),
            ..Default::default()
        },
    ];
    let mut skills = choice("juno-codex", "skills work", false);
    skills.row.workspaces = vec![
        crate::cli::interactive::session_picker::data::WorkspaceGroup {
            id: "skills-id".into(),
            name: "skills".into(),
            path: Some("/repo/skills".into()),
            ..Default::default()
        },
    ];
    let mut state = PickerState::new(vec![mosaico, skills]);

    state.handle_key(key(KeyCode::Char('p')), 10);
    state.handle_key(key(KeyCode::Down), 10);
    state.handle_key(key(KeyCode::Enter), 10);

    assert_eq!(state.project_filter.as_deref(), Some("mosaico-id"));
    assert_eq!(state.visible, vec![0]);
}

#[test]
fn refresh_preserves_the_selected_session_and_updates_its_row() {
    let mut first = choice("opal-codex", "old", false);
    first.row.pubkey = "pk-opal".into();
    let mut selected = choice("juno-codex", "old", false);
    selected.row.pubkey = "pk-juno".into();
    let mut state = PickerState::new(vec![first, selected]);
    state.handle_key(key(KeyCode::Down), 10);

    let mut refreshed_first = choice("opal-codex", "new", false);
    refreshed_first.row.pubkey = "pk-opal".into();
    let mut refreshed_selected = choice("juno-codex", "new title", false);
    refreshed_selected.row.pubkey = "pk-juno".into();
    state.replace_choices(vec![refreshed_first, refreshed_selected]);

    assert_eq!(state.cursor, 1);
    let choice = state.current_choice().unwrap();
    assert_eq!(state.choices[choice].row.activity, "new title");
}

#[test]
fn filtering_uses_hidden_fields_and_prefers_handle_matches() {
    let mut cwd = choice("opal", "", false);
    cwd.row.cwd = Some("/repo/mosaico".into());
    let mut state = PickerState::new(vec![
        cwd,
        choice("delta-codex", "ordinary work", false),
        choice("other-codex", "reviewing delta output", false),
    ]);

    for character in "rpmosaico".chars() {
        state.handle_key(key(KeyCode::Char(character)), 10);
    }
    assert_eq!(state.visible, vec![0]);

    for _ in 0..9 {
        state.handle_key(key(KeyCode::Backspace), 10);
    }
    for character in "delta".chars() {
        state.handle_key(key(KeyCode::Char(character)), 10);
    }
    assert_eq!(state.visible[0], 1);
}

#[test]
fn cursor_scrolls_by_logical_two_line_items() {
    let choices = (0..8)
        .map(|index| choice(&format!("session-{index}"), "", false))
        .collect();
    let mut state = PickerState::new(choices);
    for _ in 0..5 {
        state.handle_key(key(KeyCode::Down), 3);
    }
    assert_eq!(state.cursor, 5);
    assert_eq!(state.offset, 3);
    assert_eq!(state.window(3).count(), 3);
}

#[test]
fn renderer_gives_every_session_exactly_two_lines() {
    let state = PickerState::new(vec![choice("one", "current activity", true)]);
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 80, 12)),
        },
    )
    .unwrap();

    let completed = terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let rows = completed
        .buffer
        .content()
        .chunks(80)
        .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>();

    assert!(rows[0].starts_with("Sessions"));
    assert!(rows[1].starts_with("❯ ● @one"));
    assert!(rows[2].starts_with("    (untitled)"));
    assert!(rows[3..11].iter().all(|row| row.trim().is_empty()));
    assert!(rows[11].starts_with("enter attach"));
}

#[test]
fn renderer_keeps_takeover_confirmation_controls_visible() {
    let mut state = PickerState::new(vec![takeover_choice("echo-codex", false)]);
    state.handle_key(key(KeyCode::Enter), 10);
    let backend = TestBackend::new(100, 12);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 100, 12)),
        },
    )
    .unwrap();

    let completed = terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let footer = completed.buffer.content()[1100..1200]
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(footer.starts_with("[y] take over  [n] cancel"), "{footer}");
}
