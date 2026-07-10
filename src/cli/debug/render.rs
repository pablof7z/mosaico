use super::data::{DebugKind, DebugLine, HookTailSnapshot, RootPopup, SessionPane};
use super::util::*;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

pub(super) struct HookTailState {
    pub(super) root_filters: std::collections::BTreeSet<String>,
    pub(super) session_filter: Option<String>,
    pub(super) pane_limit: usize,
    pub(super) focused: usize,
    pub(super) focused_session: Option<String>, // session ID of the focused pane; stable across snapshot re-sorts
    pub(super) focus_mode: bool,
    pub(super) line_cursor: usize,
    pub(super) detail_open: bool, // full-screen detail overlay for the selected line
    pub(super) status: String,
    pub(super) popup: Option<RootPopup>,
}

pub(super) fn render_hook_tail(
    f: &mut ratatui::Frame,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    let workspace_label: String = if state.root_filters.is_empty() {
        "*".to_string()
    } else {
        state
            .root_filters
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    };
    let session = state.session_filter.as_deref().unwrap_or("*");
    let title = Line::from(vec![
        Span::styled(
            "tenex-edge debug hook-tail",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  workspace="),
        Span::styled(workspace_label, Style::default().fg(Color::Yellow)),
        Span::raw("  session="),
        Span::styled(session, Style::default().fg(Color::Yellow)),
        Span::raw("  panes="),
        Span::styled(
            state.pane_limit.to_string(),
            Style::default().fg(Color::Yellow),
        ),
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    if state.focus_mode {
        render_focus(f, chunks[1], snapshot, state);
    } else {
        render_grid(f, chunks[1], snapshot, state);
    }

    let hints = if state.detail_open {
        "[any key] close"
    } else if state.popup.is_some() {
        "[↑↓] move  [space] toggle  [a] clear  [esc/p] close"
    } else if state.focus_mode {
        "[↑↓] select  [enter] open  [tab/←/→] pane  [f/esc] exit zoom  [q] quit"
    } else {
        "[enter/f] zoom  [tab/←/→] pane  [+/-] panes  [p] workspaces  [s] session  [a] clear  [q] quit"
    };
    let status = if state.status.is_empty()
        || state.popup.is_some()
        || state.focus_mode
        || state.detail_open
    {
        hints.to_string()
    } else {
        format!("{}  {}", state.status, hints)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            status,
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[2],
    );

    if state.popup.is_some() {
        render_root_popup(f, area, snapshot, state);
    }
    if state.detail_open {
        render_detail_overlay(f, area, snapshot, state);
    }
}

fn centered_rect(percent_x: u16, max_height: u16, r: Rect) -> Rect {
    let height = max_height.min(r.height.saturating_sub(4));
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((r.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(r);
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1]);
    horiz[1]
}

fn render_detail_overlay(
    f: &mut ratatui::Frame,
    area: Rect,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let pane = snapshot.panes.get(state.focused);
    let n = pane.map(|p| p.lines.len()).unwrap_or(0);
    let selected = if state.line_cursor == usize::MAX || state.line_cursor >= n {
        n.saturating_sub(1)
    } else {
        state.line_cursor
    };
    let line = pane.and_then(|p| p.lines.get(selected));
    let (label, detail) = match line {
        Some(l) => (l.label.as_str(), l.detail.as_str()),
        None => ("detail", ""),
    };

    let overlay = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(3),
    };
    f.render_widget(Clear, overlay);

    let text_lines: Vec<Line> = detail
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::White),
            ))
        })
        .collect();

    f.render_widget(
        Paragraph::new(text_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(format!(" {label}  [any key] close ")),
            )
            .wrap(Wrap { trim: false }),
        overlay,
    );
}

fn render_root_popup(
    f: &mut ratatui::Frame,
    area: Rect,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let Some(popup) = &state.popup else { return };
    let popup_area = centered_rect(50, (snapshot.roots.len() as u16 + 4).max(6), area);
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = snapshot
        .roots
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let checked = state.root_filters.contains(p);
            let focused = i == popup.cursor;
            let prefix = if checked { " [x] " } else { " [ ] " };
            let style = if focused {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if checked {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!("{}{}", prefix, p)).style(style)
        })
        .collect();

    let title = if state.root_filters.is_empty() {
        " Workspaces (all) ".to_string()
    } else {
        format!(" Workspaces ({} selected) ", state.root_filters.len())
    };

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(title),
        ),
        popup_area,
    );
}

fn render_grid(
    f: &mut ratatui::Frame,
    area: Rect,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let count = snapshot.panes.len().min(state.pane_limit);
    if count == 0 {
        render_empty(f, area, snapshot);
        return;
    }
    let rects = grid_rects(area, count);
    for (i, rect) in rects.into_iter().enumerate() {
        if let Some(pane) = snapshot.panes.get(i) {
            render_pane_grid(f, rect, pane, i == state.focused);
        }
    }
}

fn render_focus(
    f: &mut ratatui::Frame,
    area: Rect,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let Some(pane) = snapshot.panes.get(state.focused) else {
        render_empty(f, area, snapshot);
        return;
    };

    let n = pane.lines.len();
    let selected = if state.line_cursor == usize::MAX || state.line_cursor >= n {
        n.saturating_sub(1)
    } else {
        state.line_cursor
    };

    // Height of detail panel: clamp to [4, 12] based on content
    let selected_line = pane.lines.get(selected);
    let detail_line_count = selected_line
        .map(|l| l.detail.lines().count().max(1))
        .unwrap_or(1);
    let detail_height = (detail_line_count as u16 + 2).clamp(4, 12);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(detail_height)])
        .split(area);

    render_pane_focus(f, chunks[0], pane, selected);
    render_detail_panel(f, chunks[1], selected_line);
}

fn render_empty(f: &mut ratatui::Frame, area: Rect, snapshot: &HookTailSnapshot) {
    let mut lines = vec![Line::from("No session telemetry yet.")];
    if !snapshot.unscoped.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from("Recent unscoped commands:"));
        let base = snapshot.unscoped.first().map(|l| l.ts_ms).unwrap_or(0);
        for line in snapshot.unscoped.iter().rev().take(8) {
            lines.push(render_timeline_line(line, base, false));
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("debug")),
        area,
    );
}

fn pane_title(pane: &SessionPane) -> String {
    match (pane.agent.as_str(), pane.root.as_str()) {
        ("", "") => pane.short.clone(),
        (agent, "") => format!("{} [{}]", agent, pane.short),
        ("", root) => format!("{} [{}]", root, pane.short),
        (agent, root) => format!("{}@{} [{}]", agent, root, pane.short),
    }
}

fn render_pane_grid(f: &mut ratatui::Frame, area: Rect, pane: &SessionPane, focused: bool) {
    let border_color = if focused {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let inner_h = area.height.saturating_sub(2) as usize;
    let base_ts = pane.lines.first().map(|l| l.ts_ms).unwrap_or(0);
    let start = pane.lines.len().saturating_sub(inner_h);
    let lines: Vec<Line> = pane
        .lines
        .iter()
        .skip(start)
        .map(|l| render_timeline_line(l, base_ts, false))
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(pane_title(pane)),
        ),
        area,
    );
}

fn render_pane_focus(f: &mut ratatui::Frame, area: Rect, pane: &SessionPane, selected: usize) {
    let inner_h = area.height.saturating_sub(2) as usize;
    let n = pane.lines.len();
    // Scroll to keep selected visible
    let scroll = if selected < inner_h {
        0
    } else {
        selected - inner_h + 1
    };
    let base_ts = pane.lines.first().map(|l| l.ts_ms).unwrap_or(0);
    let lines: Vec<Line> = pane
        .lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(inner_h)
        .map(|(i, l)| render_timeline_line(l, base_ts, i == selected))
        .collect();
    let title = format!("{} ({}/{})", pane_title(pane), selected + 1, n);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(title),
        ),
        area,
    );
}

fn render_detail_panel(f: &mut ratatui::Frame, area: Rect, line: Option<&DebugLine>) {
    let (label, text, color) = match line {
        None => ("detail", String::new(), Color::DarkGray),
        Some(l) => {
            let c = kind_color(l.kind);
            (l.label.as_str(), l.detail.clone(), c)
        }
    };
    let text_lines: Vec<Line> = text
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::White),
            ))
        })
        .collect();
    f.render_widget(
        Paragraph::new(text_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(color))
                    .title(format!(" {label} ")),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_timeline_line(line: &DebugLine, base_ts: u128, selected: bool) -> Line<'static> {
    let ts = fmt_rel_ts(line.ts_ms, base_ts);
    let label = fixed_label(&line.label, 18);
    let color = kind_color(line.kind);
    let bg = if selected {
        Color::Rgb(40, 40, 60)
    } else {
        Color::Reset
    };

    // User prompt text gets a brighter color so it's easy to scan
    let summary_color = match line.kind {
        DebugKind::Hook if line.label == "user-prompt-submit" => Color::LightYellow,
        DebugKind::Hook
            if line.label.starts_with("pre-tool-use")
                || line.label.starts_with("post-tool-use") =>
        {
            Color::Gray
        }
        DebugKind::Inject => Color::Gray,
        DebugKind::Command => Color::LightCyan,
        DebugKind::Error => Color::LightRed,
        _ => Color::Gray,
    };

    let base = Style::default().bg(bg);
    Line::from(vec![
        Span::styled(ts, base.fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(label, base.fg(color).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(line.summary.clone(), base.fg(summary_color)),
    ])
}

fn grid_rects(area: Rect, count: usize) -> Vec<Rect> {
    let cols = (count as f64).sqrt().ceil() as usize;
    let cols = cols.max(1);
    let rows = count.div_ceil(cols).max(1);
    let row_constraints = even_constraints(rows);
    let col_constraints = even_constraints(cols);
    let row_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(area);
    let mut rects = Vec::new();
    for row in row_rects.iter() {
        let cols_rects = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints.clone())
            .split(*row);
        for col in cols_rects.iter() {
            if rects.len() < count {
                rects.push(*col);
            }
        }
    }
    rects
}

fn even_constraints(n: usize) -> Vec<Constraint> {
    (0..n)
        .map(|_| Constraint::Ratio(1, n as u32))
        .collect::<Vec<_>>()
}
