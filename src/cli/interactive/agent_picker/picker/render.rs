use super::{AgentPickerRow, DeleteScope, PendingDelete, PickerState};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, Paragraph},
    Frame,
};

const ACCENT: Color = Color::Indexed(45);
const MUTED: Color = Color::Indexed(245);
const ERROR: Color = Color::Indexed(203);

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

    let query = if state.query.is_empty() {
        Span::styled("type to filter", Style::default().fg(MUTED))
    } else {
        Span::styled(state.query.as_str(), Style::default().fg(ACCENT))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Agents", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  Filter: "),
            query,
        ])),
        title_area,
    );

    const CARET_WIDTH: usize = 2; // "❯ " / "  "
    const MARKER_WIDTH: usize = 2; // "✓ " / "  " multi-select marker
    const PREFIX_WIDTH: usize = CARET_WIDTH + MARKER_WIDTH;
    const GAP_WIDTH: usize = 2; // spacing between name and description columns
    const RIGHT_MARGIN: usize = 1; // keeps text off the terminal's raw edge
    const MAX_NAME_WIDTH: usize = 24;

    // Sized from the currently visible (filtered) rows, so filtering down to
    // short names shrinks the gap instead of leaving it fixed to the widest
    // name in the whole roster.
    let name_width = state
        .visible
        .iter()
        .map(|&index| state.rows[index].name.chars().count())
        .max()
        .unwrap_or(0)
        .min(MAX_NAME_WIDTH);
    let description_width = usize::from(options_area.width)
        .saturating_sub(PREFIX_WIDTH + name_width + GAP_WIDTH + RIGHT_MARGIN);
    // Unconfigured native-profile-only agents are visually separated from
    // the rest by a blank line — but only in the natural (unfiltered)
    // order; fuzzy-filtered results are sorted by relevance instead, where
    // a group boundary wouldn't mean anything.
    let show_groups = state.query.is_empty();
    let mut items = Vec::with_capacity(usize::from(options_area.height));
    for (visible_index, row) in state.window(usize::from(options_area.height)) {
        if show_groups && visible_index > 0 {
            let previous = &state.rows[state.visible[visible_index - 1]];
            if !is_profile_only(previous) && is_profile_only(row) {
                items.push(ListItem::new(Line::from("")));
            }
        }
        let focused = visible_index == state.cursor;
        let selected = state.selected.contains(&state.visible[visible_index]);
        let name = truncate(&row.name, name_width);
        let description = truncate(&row.description, description_width);
        let spans = vec![
            Span::styled(
                if focused { "❯ " } else { "  " },
                Style::default().fg(if focused { ACCENT } else { MUTED }),
            ),
            Span::styled(
                if selected { "✓ " } else { "  " },
                Style::default().fg(if selected { ACCENT } else { MUTED }),
            ),
            Span::styled(
                format!("{name:<name_width$}"),
                Style::default()
                    .fg(if focused { ACCENT } else { Color::White })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(description, Style::default().fg(MUTED)),
        ];
        items.push(ListItem::new(Line::from(spans)));
    }
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
    let line = if let Some(notice) = delete_notice(state) {
        Line::from(Span::styled(
            format!("{notice} · {position}"),
            Style::default().fg(ERROR),
        ))
    } else {
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
        Line::from(status)
    };
    frame.render_widget(Paragraph::new(line), help_area);
}

fn delete_notice(state: &PickerState) -> Option<String> {
    match state.pending_delete.as_ref()? {
        PendingDelete::Nothing { index } => Some(format!(
            "{} is a generic agent — nothing to delete · any key cancels",
            state.rows[*index].name
        )),
        PendingDelete::ChooseScope { index } => Some(format!(
            "Delete {}: a) agent config · p) native profile · b) both · esc cancel",
            state.rows[*index].name
        )),
        PendingDelete::Confirm { plan } => {
            let what = if let [(index, scope)] = plan.as_slice() {
                let target = match scope {
                    DeleteScope::Agent => "agent configuration",
                    DeleteScope::Profile => "native profile",
                    DeleteScope::Both => "agent configuration and native profile",
                };
                format!("{target} for {}", state.rows[*index].name)
            } else {
                format!("{} selected agents", plan.len())
            };
            Some(format!("Delete {what}? y/d confirm · esc cancel"))
        }
    }
}

fn is_profile_only(row: &AgentPickerRow) -> bool {
    !row.has_configured && row.has_native_profile
}

fn help(state: &PickerState) -> &'static str {
    if !state.query.is_empty() {
        "enter launch · type filter · ↑↓ move · esc clear"
    } else {
        "enter launch · ctrl-e edit · ctrl-d delete · ctrl-space select · type filter · ↑↓ move · esc"
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
