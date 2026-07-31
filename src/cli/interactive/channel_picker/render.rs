use super::state::{Pending, PickerState};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, Paragraph},
    Frame,
};

const ACCENT: Color = Color::Indexed(45);
const SELECTED_BG: Color = Color::Indexed(236);
const MUTED: Color = Color::Indexed(245);
const ERROR: Color = Color::Indexed(203);
const WARNING: Color = Color::Indexed(214);

pub(super) fn draw(frame: &mut Frame<'_>, state: &PickerState) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    if area.height < 3 {
        frame.render_widget(Paragraph::new("Channels"), area);
        return;
    }

    let [title_area, options_area, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let query = if state.query().is_empty() {
        Span::styled("type to search", Style::default().fg(MUTED))
    } else {
        Span::styled(state.query(), Style::default().fg(ACCENT))
    };
    let title = Line::from(vec![
        Span::styled("Channels", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(
            format!("{} channels", state.row_count()),
            Style::default().fg(MUTED),
        ),
        Span::styled(
            "  Search: ",
            if state.query().is_empty() {
                Style::default().fg(MUTED)
            } else {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            },
        ),
        query,
    ]);
    frame.render_widget(Paragraph::new(title), title_area);

    let width = usize::from(options_area.width.saturating_sub(4));
    let items = state
        .window(usize::from(options_area.height))
        .map(|(position, row)| {
            let focused = position == state.cursor_index();
            let tree = if row.has_children {
                if row.expanded {
                    "▾ "
                } else {
                    "▸ "
                }
            } else if row.depth > 0 {
                "· "
            } else {
                "  "
            };
            let indent = "  ".repeat(row.depth);
            let mut details = Vec::new();
            if !row.about.is_empty() {
                details.push(row.about.clone());
            }
            if let Some(agents) = row.agents {
                details.push(format!(
                    "{agents} agent{}",
                    if agents == 1 { "" } else { "s" }
                ));
            }
            if let Some(activity) = row.last_activity.as_deref() {
                details.push(format!("active {activity}"));
            }
            let detail = if details.is_empty() {
                String::new()
            } else {
                format!("  — {}", details.join(" · "))
            };
            let line = format!("{indent}{tree}{}{detail}", row.path);
            ListItem::new(Line::from(vec![
                caret(focused),
                Span::styled(
                    truncate(&line, width),
                    if focused {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]))
            .style(if focused {
                Style::default().bg(SELECTED_BG)
            } else {
                Style::default()
            })
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("  No matching channels").style(Style::default().fg(MUTED)),
            options_area,
        );
    } else {
        frame.render_widget(List::new(items), options_area);
    }

    let footer = if let Some(Pending::ConfirmDelete { path }) = state.pending() {
        (
            format!("Delete {path}? This sends kind:9008 · y confirm · n/esc cancel"),
            ERROR,
        )
    } else if let Some(notice) = state.notice() {
        (notice.to_string(), WARNING)
    } else {
        (help(state).to_string(), MUTED)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{} · {}", footer.0, state.position_label()),
            Style::default().fg(footer.1),
        ))),
        help_area,
    );
}

fn help(state: &PickerState) -> &'static str {
    if !state.query().is_empty() {
        return "e edit · d delete · type search · ↑↓ move · esc clear";
    }
    "e edit about · d delete · ←/→ tree · type search · ↑↓ move · ctrl-r refresh · q quit"
}

fn caret(focused: bool) -> Span<'static> {
    Span::styled(
        if focused { "❯ " } else { "  " },
        Style::default().fg(if focused { ACCENT } else { MUTED }),
    )
}

fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let kept: String = text.chars().take(width - 1).collect();
    format!("{kept}…")
}
