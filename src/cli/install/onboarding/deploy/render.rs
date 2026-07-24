//! Render the relay-assist modal in Mosaico's own visual language.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::super::theme::{self, ACCENT, ACCENT_ALT, ERR, FAINT, MUTED, OK, WARN};
use super::session::DeploySession;
use super::transcript::{DeployStatus, Entry};

pub(in crate::cli::install::onboarding) fn draw(frame: &mut Frame, session: &DeploySession) {
    let area = frame.area();
    let [header, body, strip, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(area);

    render_header(frame, header);
    render_transcript(frame, body, session);
    render_strip(frame, strip, session);
    render_footer(frame, footer, session);

    if session.pending().is_some() {
        render_permission(frame, area, session);
    }
}

fn render_header(frame: &mut Frame, area: Rect) {
    let title = Line::from(vec![
        Span::styled("◆ MOSAICO", theme::bold(ACCENT)),
        Span::styled("  ·  ", theme::fg(FAINT)),
        Span::styled("relay assist", theme::fg(MUTED)),
    ]);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme::fg(FAINT));
    frame.render_widget(Paragraph::new(title).block(block), area);
}

fn render_transcript(frame: &mut Frame, area: Rect, session: &DeploySession) {
    let inner = Layout::horizontal([Constraint::Percentage(100)])
        .horizontal_margin(1)
        .areas::<1>(area)[0];
    let lines: Vec<Line> = session.entries().iter().map(entry_line).collect();
    // Auto-scroll: keep the tail visible.
    let overflow = lines.len().saturating_sub(inner.height as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((overflow, 0)),
        inner,
    );
}

fn entry_line(entry: &Entry) -> Line<'static> {
    match entry {
        Entry::Agent(t) => Line::from(Span::styled(
            t.clone(),
            theme::fg(ratatui::style::Color::Reset),
        )),
        Entry::Thought(t) => Line::from(Span::styled(format!("  {t}"), theme::fg(FAINT))),
        Entry::Activity(t) => Line::from(vec![
            Span::styled("⚙ ", theme::fg(ACCENT)),
            Span::styled(t.clone(), theme::fg(MUTED)),
        ]),
        Entry::Notice(t) => Line::from(Span::styled(format!("· {t}"), theme::fg(FAINT))),
        Entry::Error(t) => Line::from(Span::styled(format!("✗ {t}"), theme::fg(ERR))),
    }
}

fn render_strip(frame: &mut Frame, area: Rect, session: &DeploySession) {
    let (label, color) = match session.status() {
        DeployStatus::Connecting => ("connecting…", ACCENT),
        DeployStatus::Working => ("agent working…", ACCENT),
        DeployStatus::AwaitingPermission => ("awaiting your approval", WARN),
        DeployStatus::Idle => ("waiting for relay…", MUTED),
        DeployStatus::RelayOnline => ("relay online — NIP-29 ready", OK),
        DeployStatus::Failed(_) => ("agent session failed", ERR),
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme::fg(FAINT));
    let line = Line::from(vec![
        Span::styled(format!("● {label}"), theme::fg(color)),
        Span::styled("   target ", theme::fg(FAINT)),
        Span::styled(session.relay_url().to_string(), theme::fg(ACCENT)),
    ]);
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn render_footer(frame: &mut Frame, area: Rect, session: &DeploySession) {
    let hint = if session.pending().is_some() {
        "↑↓ choose · Enter allow · n deny"
    } else {
        "watching the agent · Esc cancel"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, theme::fg(MUTED)))),
        area,
    );
}

fn render_permission(frame: &mut Frame, area: Rect, session: &DeploySession) {
    let Some(ask) = session.pending() else {
        return;
    };
    let width = area.width.saturating_sub(8).min(70).max(20);
    let height = (ask.options.len() as u16 + 6).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal = Rect {
        x,
        y,
        width,
        height,
    };
    frame.render_widget(Clear, modal);

    let mut lines = vec![
        Line::from(Span::styled(
            "Permission requested",
            theme::bold(ACCENT_ALT),
        )),
        Line::from(Span::styled(ask.summary.clone(), theme::fg(MUTED))),
        Line::from(""),
    ];
    for (i, opt) in ask.options.iter().enumerate() {
        let here = i == session.option_cursor();
        let pointer = if here { "❯ " } else { "  " };
        let style = if here {
            theme::bold(ACCENT)
        } else if opt.allow {
            theme::fg(OK)
        } else {
            theme::fg(MUTED)
        };
        lines.push(Line::from(vec![
            Span::styled(pointer, theme::fg(ACCENT_ALT)),
            Span::styled(opt.label.clone(), style),
        ]));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::fg(WARN))
        .title(" approve ");
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(block),
        modal,
    );
}

#[cfg(test)]
mod preview {
    use super::super::session::DeploySession;
    use super::super::transcript::{DeployEvent, Transcript};
    use super::super::{PermissionAsk, PermissionOption};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn seeded() -> Transcript {
        let mut t = Transcript::new();
        t.apply(DeployEvent::Notice("starting claude…".into()));
        t.apply(DeployEvent::Agent(
            "I'll run a Croissant relay for you on 127.0.0.1:9888.".into(),
        ));
        t.apply(DeployEvent::Activity(
            "bash: which croissant [in_progress]".into(),
        ));
        t
    }

    fn render(session: &DeploySession) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal.draw(|f| super::draw(f, session)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let area = *buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn preview_modal_working_and_permission() {
        let working = DeploySession::for_preview(seeded(), None, "ws://127.0.0.1:9888");
        println!("\n┌─ MODAL: working {}", "─".repeat(60));
        for line in render(&working).lines() {
            println!("│{line}");
        }

        let (respond, _rx) = std::sync::mpsc::channel();
        let ask = PermissionAsk {
            summary: "Run: brew install croissant".into(),
            options: vec![
                PermissionOption {
                    id: "allow".into(),
                    label: "Allow once".into(),
                    allow: true,
                },
                PermissionOption {
                    id: "deny".into(),
                    label: "Deny".into(),
                    allow: false,
                },
            ],
            respond,
        };
        let asking = DeploySession::for_preview(seeded(), Some(ask), "ws://127.0.0.1:9888");
        println!("\n┌─ MODAL: permission {}", "─".repeat(57));
        for line in render(&asking).lines() {
            println!("│{line}");
        }
    }
}
