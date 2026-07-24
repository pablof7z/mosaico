//! Ratatui rendering for the onboarding TUI and the terminal-state guard.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::model::{Onboarding, RelayChoice, RelayStatus, Step};
use super::theme::{self, ACCENT, ACCENT_ALT, ERR, FAINT, MUTED, OK, WARN};

const MIN_W: u16 = 56;
const MIN_H: u16 = 16;

#[cfg(test)]
#[path = "render_preview.rs"]
mod preview;

/// RAII guard: enters the alternate screen + raw mode, and always restores the
/// terminal on drop (including during panic unwind).
pub(super) struct TuiTerminal;

impl TuiTerminal {
    pub(super) fn enter() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        )?;
        Ok(Self)
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::cursor::Show,
            crossterm::terminal::LeaveAlternateScreen
        );
    }
}

pub(super) fn draw(frame: &mut Frame, state: &Onboarding) {
    let area = frame.area();
    if area.width < MIN_W || area.height < MIN_H {
        let msg = format!("Mosaico setup needs at least {MIN_W}×{MIN_H}. Enlarge the terminal.");
        frame.render_widget(Paragraph::new(msg).wrap(Wrap { trim: true }), area);
        return;
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(area);

    render_header(frame, header, state);
    render_body(frame, body, state);
    render_footer(frame, footer, state);
}

fn render_header(frame: &mut Frame, area: Rect, state: &Onboarding) {
    let brand = Span::styled("◆ MOSAICO", theme::bold(ACCENT));
    let dot = Span::styled("  ·  ", theme::fg(FAINT));
    let tag = Span::styled("first-run setup", theme::fg(MUTED));
    let title = Line::from(vec![brand, dot, tag]);
    let breadcrumb = breadcrumb_line(state);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme::fg(FAINT));
    frame.render_widget(Paragraph::new(vec![title, breadcrumb]).block(block), area);
}

const STEPS: [(Step, &str); 5] = [
    (Step::Identity, "identity"),
    (Step::DeviceName, "device"),
    (Step::Harnesses, "harnesses"),
    (Step::Relay, "relay"),
    (Step::Review, "review"),
];

fn step_rank(step: Step) -> usize {
    match step {
        Step::Splash | Step::Identity => 0,
        Step::DeviceName => 1,
        Step::Harnesses => 2,
        Step::Relay | Step::RelayUrl | Step::Deploy => 3,
        Step::Review => 4,
    }
}

fn breadcrumb_line(state: &Onboarding) -> Line<'static> {
    let here = step_rank(state.step);
    let mut spans = Vec::new();
    for (i, (_, label)) in STEPS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" → ", theme::fg(FAINT)));
        }
        let style = if i == here {
            theme::bold(ACCENT_ALT)
        } else if i < here {
            theme::fg(OK)
        } else {
            theme::fg(FAINT)
        };
        spans.push(Span::styled(*label, style));
    }
    Line::from(spans)
}

fn render_body(frame: &mut Frame, area: Rect, state: &Onboarding) {
    let inner = Layout::horizontal([Constraint::Percentage(100)])
        .horizontal_margin(2)
        .areas::<1>(area)[0];
    let lines = match state.step {
        Step::Splash => splash(state),
        Step::Identity => identity(state),
        Step::DeviceName => device_name(state),
        Step::Harnesses => harnesses(state),
        Step::Relay => relay(state),
        Step::RelayUrl => relay_url(state),
        Step::Deploy => vec![muted("Launching the agent…")],
        Step::Review => review(state),
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn heading(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), theme::bold(ACCENT)))
}

fn blank() -> Line<'static> {
    Line::from("")
}

fn muted(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(text.into(), theme::fg(MUTED)))
}

fn splash(state: &Onboarding) -> Vec<Line<'static>> {
    let shimmer = if !state.reduced && state.frame % 2 == 1 {
        ACCENT_ALT
    } else {
        ACCENT
    };
    vec![
        blank(),
        Line::from(Span::styled("◆  M O S A I C O", theme::bold(shimmer))),
        blank(),
        muted("A self-organizing society of agents, serving your intent."),
        blank(),
        muted("Looking over this machine…"),
    ]
}

fn identity(state: &Onboarding) -> Vec<Line<'static>> {
    vec![
        heading("Your operator identity"),
        blank(),
        muted("Generated for you. It signs your commands and is saved to config.json."),
        blank(),
        kv("npub", &state.identity.npub, ACCENT),
        kv("nsec", &state.identity.nsec, WARN),
        blank(),
        muted("This nsec is your secret key. It is stored locally; keep it private."),
    ]
}

fn kv(key: &str, value: &str, value_color: ratatui::style::Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {key:<8}"), theme::fg(FAINT)),
        Span::styled(value.to_string(), theme::fg(value_color)),
    ])
}

fn device_name(state: &Onboarding) -> Vec<Line<'static>> {
    vec![
        heading("Name this device"),
        blank(),
        muted("The label other agents see for this backend. Defaults from the hostname."),
        blank(),
        Line::from(vec![
            Span::styled("  ▏", theme::fg(ACCENT)),
            Span::styled(state.device_name.clone(), theme::bold(ACCENT)),
            Span::styled("▏", theme::fg(ACCENT)),
        ]),
    ]
}

fn harnesses(state: &Onboarding) -> Vec<Line<'static>> {
    let mut lines = vec![
        heading("Choose harness integrations"),
        blank(),
        muted("Detected on your PATH are pre-selected. Space toggles, Enter continues."),
        blank(),
    ];
    for (i, h) in state.all.iter().enumerate() {
        let on = state.selected.get(i).copied().unwrap_or(false);
        let here = i == state.cursor;
        let mark = if on { "◉" } else { "○" };
        let pointer = if here { "❯ " } else { "  " };
        let installed = super::super::is_installed(h);
        let status = if installed {
            "installed"
        } else if h.detected {
            "detected"
        } else {
            "not detected"
        };
        let status_color = if installed || h.detected { OK } else { FAINT };
        let name_style = if here {
            theme::bold(ACCENT_ALT)
        } else if on {
            theme::fg(ACCENT)
        } else {
            theme::fg(MUTED)
        };
        lines.push(Line::from(vec![
            Span::styled(pointer, theme::fg(ACCENT_ALT)),
            Span::styled(
                format!("{mark} "),
                theme::fg(if on { ACCENT } else { FAINT }),
            ),
            Span::styled(format!("{:<14}", h.display), name_style),
            Span::styled(status.to_string(), theme::fg(status_color)),
        ]));
    }
    lines
}

fn relay(state: &Onboarding) -> Vec<Line<'static>> {
    let mut lines = vec![
        heading("Where should the relay run?"),
        blank(),
        muted("Your agents coordinate through a shared relay. Pick how to provide one."),
        blank(),
    ];
    for (i, choice) in RelayChoice::ALL.iter().enumerate() {
        let here = i == state.relay_cursor;
        let pointer = if here { "❯ " } else { "  " };
        let title_style = if here {
            theme::bold(ACCENT_ALT)
        } else {
            theme::fg(ACCENT)
        };
        lines.push(Line::from(vec![
            Span::styled(pointer, theme::fg(ACCENT_ALT)),
            Span::styled(choice.title().to_string(), title_style),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", theme::fg(FAINT)),
            Span::styled(choice.blurb().to_string(), theme::fg(MUTED)),
        ]));
    }
    if let RelayStatus::Failed(msg) = &state.relay_status {
        lines.push(blank());
        lines.push(Line::from(Span::styled(
            format!("  ! {msg}"),
            theme::fg(WARN),
        )));
    }
    lines
}

fn relay_url(state: &Onboarding) -> Vec<Line<'static>> {
    let (title, help) = match state.relay_choice() {
        RelayChoice::Existing => (
            "Connect an existing relay",
            "Enter a ws:// or wss:// URL. It must be reachable and announce NIP-29.",
        ),
        RelayChoice::Manual => (
            "Where will your relay run?",
            "Enter the ws:// or wss:// URL you'll serve Croissant on.",
        ),
        RelayChoice::Assist => (
            "Where should the agent put the relay?",
            "Enter the ws:// or wss:// URL the agent should make reachable.",
        ),
    };
    let mut lines = vec![
        heading(title),
        blank(),
        muted(help),
        blank(),
        Line::from(vec![
            Span::styled("  ▏", theme::fg(ACCENT)),
            Span::styled(state.relay_url.clone(), theme::bold(ACCENT)),
            Span::styled("▏", theme::fg(ACCENT)),
        ]),
        blank(),
    ];
    lines.push(status_line(&state.relay_status));
    lines
}

fn status_line(status: &RelayStatus) -> Line<'static> {
    match status {
        RelayStatus::Idle => blank(),
        RelayStatus::Verifying => Line::from(Span::styled("  ⟳ verifying…", theme::fg(ACCENT))),
        RelayStatus::Usable => {
            Line::from(Span::styled("  ✓ reachable, NIP-29 ready", theme::fg(OK)))
        }
        RelayStatus::Warn(m) => Line::from(Span::styled(format!("  ! {m}"), theme::fg(WARN))),
        RelayStatus::Failed(m) => Line::from(Span::styled(format!("  ✗ {m}"), theme::fg(ERR))),
    }
}

fn review(state: &Onboarding) -> Vec<Line<'static>> {
    let chosen: Vec<&str> = state
        .all
        .iter()
        .zip(&state.selected)
        .filter(|(_, on)| **on)
        .map(|(h, _)| h.display)
        .collect();
    let harness_summary = if chosen.is_empty() {
        "none".to_string()
    } else {
        chosen.join(", ")
    };
    let relay_summary = match state.relay_choice() {
        RelayChoice::Existing => state.relay_url.trim().to_string(),
        RelayChoice::Assist => format!("{}  (set up with an agent)", state.relay_url.trim()),
        RelayChoice::Manual => format!("{}  (you'll run Croissant)", state.relay_url.trim()),
    };
    vec![
        heading("Ready to set up Mosaico"),
        blank(),
        kv("device", &state.device_name, ACCENT),
        kv("npub", &state.identity.npub, MUTED),
        kv("relay", &relay_summary, ACCENT),
        kv("agents", &harness_summary, ACCENT),
        blank(),
        muted("Press Enter to apply. Mosaico will write config.json, wire the"),
        muted("selected harnesses, install the skill, and bring the relay online."),
    ]
}

fn render_footer(frame: &mut Frame, area: Rect, state: &Onboarding) {
    let hint = match state.step {
        Step::Splash => "any key to begin",
        Step::Identity => "Enter continue · Esc quit",
        Step::DeviceName => "type to edit · Enter continue · Esc back",
        Step::Harnesses => "↑↓ move · Space toggle · Enter continue · Esc back",
        Step::Relay => "↑↓ move · Enter select · Esc back",
        Step::RelayUrl => "type URL · Enter continue · Esc back",
        Step::Deploy => "watching the agent · Esc cancel",
        Step::Review => "Enter apply · Esc back",
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme::fg(FAINT));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, theme::fg(MUTED))))
            .block(block)
            .alignment(Alignment::Left),
        area,
    );
}
