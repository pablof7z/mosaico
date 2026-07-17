use super::PickerState;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, Paragraph},
    Frame,
};

const ACCENT: Color = Color::Indexed(45);
const MUTED: Color = Color::Indexed(245);

pub(super) fn draw(frame: &mut Frame<'_>, state: &PickerState) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    if area.height < 3 {
        frame.render_widget(Paragraph::new("Agents"), area);
        return;
    }
    let [title_area, options_area, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let query = if state.filtering && state.query.is_empty() {
        Span::styled("type to filter", Style::default().fg(MUTED))
    } else if state.filtering {
        Span::styled(state.query.as_str(), Style::default().fg(ACCENT))
    } else {
        Span::styled("press / to filter", Style::default().fg(MUTED))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Agents", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  Filter: "),
            query,
        ])),
        title_area,
    );

    let name_width = state
        .rows
        .iter()
        .map(|row| row.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(30);
    let items = state
        .window(usize::from(options_area.height))
        .map(|(visible_index, row)| {
            let focused = visible_index == state.cursor;
            let name = truncate(&row.name, name_width);
            let spans = vec![
                Span::styled(
                    if focused { "❯ " } else { "  " },
                    Style::default().fg(if focused { ACCENT } else { MUTED }),
                ),
                Span::styled(
                    format!("{name:<name_width$}"),
                    Style::default()
                        .fg(if focused { ACCENT } else { Color::White })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(row.description.clone(), Style::default().fg(MUTED)),
            ];
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("  No matching agents").style(Style::default().fg(MUTED)),
            options_area,
        );
    } else {
        frame.render_widget(List::new(items), options_area);
    }

    let position = if state.visible.is_empty() {
        "0/0".to_string()
    } else {
        format!("{}/{}", state.cursor + 1, state.visible.len())
    };
    let mut status = Vec::new();
    if let Some(configuration) = state.current_row().and_then(|row| row.status.as_ref()) {
        status.push(Span::styled(
            configuration.label.clone(),
            Style::default().fg(crate::console_style::harness_ratatui_color(
                configuration.harness,
            )),
        ));
        status.push(Span::styled("  ·  ", Style::default().fg(MUTED)));
    }
    status.push(Span::styled(
        format!("{} · {position}", help(state)),
        Style::default().fg(MUTED),
    ));
    frame.render_widget(Paragraph::new(Line::from(status)), help_area);
}

fn help(state: &PickerState) -> &'static str {
    if state.filtering {
        "enter launch · type filter · ↑↓ move · esc clear"
    } else {
        "enter launch · e edit · d delete · / filter · ↑↓ move · esc"
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_none() {
        prefix
    } else {
        format!(
            "{}…",
            prefix
                .chars()
                .take(max_chars.saturating_sub(1))
                .collect::<String>()
        )
    }
}
