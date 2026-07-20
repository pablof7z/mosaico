use super::*;
use ratatui::{backend::TestBackend, layout::Rect, Terminal, TerminalOptions, Viewport};

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
fn presents_sessions_and_agents_as_one_home() {
    let picker = state(vec![
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

    assert!(rows[0].starts_with("Mosaico  1 sessions · 1 agents"));
    assert!(rows[1].starts_with("Sessions  1"));
    assert!(rows[2].starts_with("❯ ● @one"));
    assert!(rows[4].starts_with("Start a session  1"));
    assert!(rows[5].contains('＋'));
    assert!(rows[5].contains("codex"));
    assert!(rows[11].starts_with("enter attach"));
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
