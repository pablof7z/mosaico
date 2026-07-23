use super::*;
use crate::{
    cli::{
        agents::{AgentKind, AgentRow},
        interactive::session_picker::{data::SessionRow, HomeChoice, SessionChoice},
    },
    harness::Transport,
    session::Harness,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

mod project_filter;
mod render;

fn session_choice(handle: &str, activity: &str, attachable: bool) -> SessionChoice {
    SessionChoice {
        row: SessionRow {
            pubkey: format!("pk-{handle}"),
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

fn session(handle: &str, activity: &str, attachable: bool) -> HomeChoice {
    HomeChoice::Session(session_choice(handle, activity, attachable))
}

fn takeover(handle: &str, turn_open: bool) -> HomeChoice {
    let mut choice = session_choice(handle, "", false);
    choice.row.transport = "process".into();
    choice.row.takeover_available = true;
    choice.row.turn_open = turn_open;
    choice.row.turn_count = 7;
    HomeChoice::Session(choice)
}

fn agent(slug: &str, kind: AgentKind) -> HomeChoice {
    HomeChoice::Agent(AgentRow {
        slug: slug.into(),
        agent_slug: slug.into(),
        description: format!("Use {slug} for implementation work"),
        harness: Harness::Codex,
        bundle: None,
        transport: Some(Transport::Pty),
        profile: None,
        per_session_key: None,
        kind,
        native_profile: None,
    })
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn state(choices: Vec<HomeChoice>) -> PickerState {
    PickerState::new(choices, None)
}

#[test]
fn picker_uses_all_non_chrome_terminal_lines() {
    assert_eq!(viewport_height(50), 50);
    assert_eq!(viewport_height(1), 1);
    assert_eq!(option_lines(12), 10);
}

#[test]
fn enter_dispatches_by_focused_row_kind() {
    let mut picker = state(vec![
        session("opal", "", true),
        agent("codex", AgentKind::Generic),
    ]);
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter), 10),
        Some(PickerExit::Attach(0))
    );
    picker.handle_key(key(KeyCode::Right), 10);
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter), 10),
        Some(PickerExit::Launch(1))
    );
}

#[test]
fn headless_session_reports_why_it_cannot_attach() {
    let mut picker = state(vec![session("opal", "", false)]);
    assert_eq!(picker.handle_key(key(KeyCode::Enter), 10), None);
    assert!(picker.notice.as_deref().unwrap().contains("no live"));

    if let HomeChoice::Session(choice) = &mut picker.choices[0] {
        choice.row.transport = "acp".into();
    }
    picker.notice = None;
    picker.handle_key(key(KeyCode::Enter), 10);
    assert!(picker.notice.as_deref().unwrap().contains("ACP"));
}

#[test]
fn takeover_keeps_both_confirmation_fences() {
    let mut picker = state(vec![takeover("echo-codex", true)]);
    picker.handle_key(key(KeyCode::Enter), 10);
    assert!(picker
        .confirmation_text()
        .unwrap()
        .contains("Kill @echo-codex"));
    assert_eq!(picker.handle_key(key(KeyCode::Char('y')), 10), None);
    assert!(picker
        .confirmation_text()
        .unwrap()
        .contains("No end-of-turn hook"));
    assert_eq!(
        picker.handle_key(key(KeyCode::Char('y')), 10),
        Some(PickerExit::TakeOver(0, Some(7)))
    );
}

#[test]
fn ctrl_k_only_kills_session_rows() {
    let ctrl_k = ctrl(KeyCode::Char('k'));
    let mut picker = state(vec![
        session("one", "", true),
        agent("codex", AgentKind::Generic),
    ]);
    assert_eq!(picker.handle_key(ctrl_k, 10), Some(PickerExit::Kill(0)));
    picker.handle_key(key(KeyCode::Right), 10);
    assert_eq!(picker.handle_key(ctrl_k, 10), None);
    assert!(picker.notice.as_deref().unwrap().contains("agent rows"));
}

#[test]
fn agent_actions_require_control_while_plain_letters_search() {
    let mut picker = state(vec![
        agent("writer", AgentKind::Configured),
        agent("reviewer", AgentKind::Configured),
    ]);
    assert_eq!(picker.handle_key(key(KeyCode::Char('e')), 10), None);
    assert_eq!(picker.query, "e");

    let mut picker = state(vec![
        agent("writer", AgentKind::Configured),
        agent("reviewer", AgentKind::Configured),
    ]);
    assert_eq!(
        picker.handle_key(ctrl(KeyCode::Char('e')), 10),
        Some(PickerExit::Edit(0))
    );

    let mut picker = state(vec![
        agent("writer", AgentKind::Configured),
        agent("reviewer", AgentKind::Configured),
    ]);
    picker.handle_key(ctrl(KeyCode::Char(' ')), 10);
    picker.handle_key(key(KeyCode::Down), 10);
    picker.handle_key(ctrl(KeyCode::Char(' ')), 10);
    picker.handle_key(ctrl(KeyCode::Char('d')), 10);
    let Some(PickerExit::Delete(plan)) = picker.handle_key(key(KeyCode::Char('y')), 10) else {
        panic!("expected bulk delete");
    };
    assert_eq!(plan.len(), 2);
}

#[test]
fn generic_bulk_delete_notice_survives_focus_moving_to_a_session() {
    let mut picker = state(vec![
        agent("codex", AgentKind::Generic),
        session("opal", "", true),
    ]);
    picker.handle_key(key(KeyCode::Right), 10);
    picker.handle_key(ctrl(KeyCode::Char(' ')), 10);
    picker.handle_key(key(KeyCode::Right), 10);
    picker.handle_key(ctrl(KeyCode::Char('d')), 10);

    assert!(picker.pending_delete.as_ref().is_some_and(|pending| {
        matches!(pending, super::delete::PendingDelete::Nothing { slug } if slug == "codex")
    }));
}

#[test]
fn search_is_scoped_to_the_active_tab_and_reaches_session_history() {
    let mut exited = session_choice("juno-codex", "finished earlier", false);
    exited.row.running = false;
    exited.row.state = crate::session_state::SessionState::Offline;
    exited.row.resumable = true;
    let mut picker = state(vec![
        session("opal-codex", "working", true),
        HomeChoice::Session(exited),
        agent("writer", AgentKind::NativeProfile),
    ]);
    assert_eq!(picker.visible, vec![0]);
    for character in "juno-codex".chars() {
        picker.handle_key(key(KeyCode::Char(character)), 10);
    }
    assert_eq!(picker.visible, vec![1]);
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter), 10),
        Some(PickerExit::Resume(1))
    );

    let mut picker = state(vec![
        session("opal", "", true),
        agent("writer", AgentKind::Generic),
    ]);
    for character in "writer".chars() {
        picker.handle_key(key(KeyCode::Char(character)), 10);
    }
    assert!(picker.visible.is_empty());
    picker.handle_key(key(KeyCode::Right), 10);
    assert_eq!(picker.visible, vec![1]);

    let mut picker = state(vec![session("opal", "", true)]);
    picker.handle_key(key(KeyCode::Char('/')), 10);
    assert_eq!(picker.query, "/");
    assert!(picker.visible.is_empty());
}

#[test]
fn refresh_updates_sessions_and_preserves_agent_focus() {
    let mut picker = state(vec![
        session("opal", "old", false),
        agent("codex", AgentKind::Generic),
    ]);
    picker.handle_key(key(KeyCode::Right), 10);
    let mut refreshed = session_choice("opal", "new", false);
    refreshed.row.pubkey = "pk-opal".into();
    picker.replace_sessions(vec![refreshed]);
    assert_eq!(
        picker.choices[picker.current_choice().unwrap()].stable_id(),
        "agent:codex"
    );
    assert_eq!(picker.choices.len(), 2);
}

#[test]
fn variable_height_window_keeps_the_cursor_visible() {
    let choices = (0..8)
        .map(|index| session(&format!("session-{index}"), "", false))
        .chain(std::iter::once(agent("codex", AgentKind::Generic)))
        .collect();
    let mut picker = state(choices);
    for _ in 0..6 {
        picker.handle_key(key(KeyCode::Down), 6);
    }
    assert_eq!(picker.cursor, 6);
    assert!(picker.offset > 0);
    assert!(picker
        .window(6)
        .iter()
        .any(|entry| entry.position == picker.cursor));
}
