//! Text rasterization of each onboarding screen for visual inspection.
//! Run with: `cargo test --lib onboarding::render -- --nocapture`.

use super::super::model::{Onboarding, Step};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn fixture() -> Onboarding {
    use crate::cli::install::config::Harness;
    let mk = |id: &'static str, display: &'static str, detected: bool| Harness {
        id,
        display,
        config_path: std::path::PathBuf::from("/tmp/x"),
        detected,
    };
    Onboarding::new(vec![
        mk("claude-code", "Claude Code", true),
        mk("codex", "Codex", true),
        mk("opencode", "opencode", false),
        mk("grok", "Grok Build", false),
    ])
    .expect("fixture")
}

fn render_to_string(state: &Onboarding) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 22)).expect("terminal");
    terminal.draw(|f| super::draw(f, state)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let area = *buffer.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn preview_all_screens() {
    let mut state = fixture();
    state.relay_url = "wss://relay.example".to_string();
    let steps = [
        ("SPLASH", Step::Splash),
        ("IDENTITY", Step::Identity),
        ("DEVICE", Step::DeviceName),
        ("HARNESSES", Step::Harnesses),
        ("RELAY", Step::Relay),
        ("RELAY-URL", Step::RelayUrl),
        ("REVIEW", Step::Review),
    ];
    for (name, step) in steps {
        state.step = step;
        println!("\n┌─ {name} {}", "─".repeat(70 - name.len()));
        for line in render_to_string(&state).lines() {
            println!("│{line}");
        }
    }
}
