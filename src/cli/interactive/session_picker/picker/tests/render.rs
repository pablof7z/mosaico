use super::*;
use ratatui::{
    backend::TestBackend, layout::Rect, style::Color, Terminal, TerminalOptions, Viewport,
};

fn terminal() -> Terminal<TestBackend> {
    Terminal::with_options(
        TestBackend::new(100, 12),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 100, 12)),
        },
    )
    .unwrap()
}

#[test]
fn presents_sessions_and_agents_in_separate_tabs_with_a_highlighted_row() {
    let mut picker = state(vec![
        session("one", "current activity", true),
        agent("codex", AgentKind::Generic),
    ]);
    let mut terminal = terminal();
    let completed = terminal
        .draw(|frame| crate::cli::interactive::session_picker::picker::render::draw(frame, &picker))
        .unwrap();
    let rows = completed
        .buffer
        .content()
        .chunks(100)
        .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>();

    assert!(rows[0].starts_with("Mosaico   Sessions 1   Start a session 1"));
    assert!(rows[1].starts_with("❯ ● @one"));
    assert!(!rows.iter().any(|row| row.contains("codex")));
    assert_eq!(completed.buffer.content()[100].bg, Color::Indexed(236));
    assert!(rows[11].starts_with("enter attach"));

    picker.handle_key(key(KeyCode::Tab), 10);
    let completed = terminal
        .draw(|frame| crate::cli::interactive::session_picker::picker::render::draw(frame, &picker))
        .unwrap();
    let rows = completed
        .buffer
        .content()
        .chunks(100)
        .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>();
    assert!(rows[1].contains('＋'));
    assert!(rows[1].contains("codex"));
    assert!(rows[1].contains("Codex"));
    assert!(!rows[1].contains("generic"));
    assert!(!rows.iter().any(|row| row.contains("@one")));
    assert!(rows[11].starts_with("generic · enter launch"));
    assert!(rows[11].contains("ctrl-e edit"));
}

#[test]
fn focused_agent_kind_moves_through_the_footer_not_the_rows() {
    let mut picker = state(vec![
        agent("generic-agent", AgentKind::Generic),
        agent("configured-agent", AgentKind::Configured),
        agent("profile-agent", AgentKind::NativeProfile),
    ]);
    let mut terminal = terminal();

    for expected in ["generic", "configured", "native profile"] {
        let completed = terminal
            .draw(|frame| {
                crate::cli::interactive::session_picker::picker::render::draw(frame, &picker)
            })
            .unwrap();
        let rows = completed
            .buffer
            .content()
            .chunks(100)
            .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>();
        assert!(rows[11].starts_with(expected), "{}", rows[11]);
        assert!(!rows[1..7]
            .iter()
            .any(|row| row.contains(&format!("Codex · {expected}"))));
        picker.handle_key(key(KeyCode::Down), 10);
    }
}

#[test]
fn active_search_label_is_white() {
    let mut picker = state(vec![session("one", "current activity", true)]);
    picker.handle_key(key(KeyCode::Char('o')), 10);
    let mut terminal = terminal();
    let completed = terminal
        .draw(|frame| crate::cli::interactive::session_picker::picker::render::draw(frame, &picker))
        .unwrap();
    let title = completed.buffer.content()[..100]
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    let search = title.find("Search:").expect("search label");

    assert_eq!(completed.buffer.content()[search].fg, Color::White);
}

#[test]
fn keeps_takeover_confirmation_visible() {
    let mut picker = state(vec![takeover("echo-codex", false)]);
    picker.handle_key(key(KeyCode::Enter), 10);
    let mut terminal = terminal();
    let completed = terminal
        .draw(|frame| crate::cli::interactive::session_picker::picker::render::draw(frame, &picker))
        .unwrap();
    let footer = completed.buffer.content()[1100..1200]
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(footer.starts_with("[y] take over  [n] cancel"), "{footer}");
}
