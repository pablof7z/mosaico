use super::tui_model::{LiveRow, ResumeRow, SpawnRow};
/// TUI rendering functions and styles
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

// ── ratatui styles ────────────────────────────────────────────────────────────

fn style_bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn style_dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn style_cyan() -> Style {
    Style::default().fg(Color::Cyan)
}

fn style_cyan_bold() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn style_yellow() -> Style {
    Style::default().fg(Color::Yellow)
}

fn style_magenta() -> Style {
    Style::default().fg(Color::Magenta)
}

fn style_magenta_bold() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD)
}

fn style_selected_bg() -> Style {
    Style::default()
}

// ── ratatui render functions ──────────────────────────────────────────────────

/// Build a `Line` for a live-session row.
pub(super) fn live_row_line(row: &LiveRow, is_sel: bool) -> Line<'static> {
    let cursor = if is_sel { "► " } else { "  " };
    let label = format!("{}@{}", row.slug, row.host);
    let session_tag = format!(" [session {}]", row.session_codename);
    let status_str = if row.status.trim().is_empty() {
        "idle".to_string()
    } else {
        row.status.trim().to_string()
    };
    if !row.attachable {
        Line::from(vec![
            Span::raw(cursor.to_string()),
            Span::styled(label, style_dim()),
            Span::styled(session_tag, style_dim()),
            Span::styled(format!("  {}", status_str), style_dim()),
        ])
    } else if is_sel {
        Line::from(vec![
            Span::styled(cursor.to_string(), style_selected_bg()),
            Span::styled(label, style_cyan_bold()),
            Span::styled(session_tag, style_yellow()),
            Span::raw(format!("  {}", status_str)),
        ])
    } else {
        Line::from(vec![
            Span::raw(cursor.to_string()),
            Span::styled(label, style_cyan()),
            Span::styled(session_tag, style_yellow()),
            Span::styled(format!("  {}", status_str), style_dim()),
        ])
    }
}

/// Build a `Line` for a spawnable-agent row.
pub(super) fn spawn_row_line(row: &SpawnRow, is_sel: bool) -> Line<'static> {
    let cursor = if is_sel { "► " } else { "  " };
    let label = format!("{}@{}", row.slug, row.host);
    let tag = format!("  [{}]", row.command);
    if is_sel {
        Line::from(vec![
            Span::raw(cursor.to_string()),
            Span::styled(label, style_bold()),
            Span::styled(tag, style_dim()),
        ])
    } else {
        Line::from(vec![
            Span::raw(cursor.to_string()),
            Span::styled(label, style_dim()),
            Span::styled(tag, style_dim()),
        ])
    }
}

/// Build a `Line` for a resumable-session row.
pub(super) fn resume_row_line(row: &ResumeRow, is_sel: bool) -> Line<'static> {
    let cursor = if is_sel { "► " } else { "  " };
    let label = row.slug.clone();
    let session_tag = format!(" [session {}]", row.session_codename);
    let title = if row.title.trim().is_empty() {
        String::new()
    } else {
        format!("  {}", row.title.trim())
    };
    if is_sel {
        Line::from(vec![
            Span::raw(cursor.to_string()),
            Span::styled(label, style_magenta_bold()),
            Span::styled(session_tag, style_yellow()),
            Span::raw(title),
        ])
    } else {
        Line::from(vec![
            Span::raw(cursor.to_string()),
            Span::styled(label, style_magenta()),
            Span::styled(session_tag, style_dim()),
            Span::styled(title, style_dim()),
        ])
    }
}

/// Render the main TUI into a ratatui `Frame`.
pub(super) fn render_main(
    f: &mut Frame,
    data: &super::tui_model::TuiData,
    selected: usize,
    status: &str,
    tabs: &[String],
    tab_idx: usize,
    exited_hours: Option<u64>,
) {
    let area = f.area();

    let project_filter = match tabs.get(tab_idx) {
        Some(p) => p.as_str(),
        None => {
            // No tabs at all — render empty.
            let lines = vec![Line::from(vec![
                Span::raw("  "),
                Span::styled("(no projects)", style_dim()),
            ])];
            f.render_widget(Paragraph::new(lines), area);
            return;
        }
    };

    // ── layout ────────────────────────────────────────────────────────────
    // Fixed rows: title (1) + tab bar (1) + rule (1) + blank (1) = 4 top chrome
    // help (1) + optional status (0 or 1) = 1–2 bottom chrome
    let bottom_chrome = if status.is_empty() { 1u16 } else { 2u16 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // title
            Constraint::Length(1),             // tab bar
            Constraint::Length(1),             // rule
            Constraint::Length(1),             // blank
            Constraint::Min(1),                // body (scrollable list)
            Constraint::Length(1),             // blank before help
            Constraint::Length(bottom_chrome), // help + optional status
        ])
        .split(area);

    // ── title ─────────────────────────────────────────────────────────────
    let title_line = Line::from(vec![Span::styled("tenex-edge tmux", style_bold())]);
    f.render_widget(Paragraph::new(title_line), chunks[0]);

    // ── tab bar ───────────────────────────────────────────────────────────
    let mut tab_spans: Vec<Span> = vec![Span::raw("  ")];
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            tab_spans.push(Span::raw(" "));
        }
        let label = format!("[{tab}]");
        if tab_idx == i {
            tab_spans.push(Span::styled(label, style_bold()));
        } else {
            tab_spans.push(Span::styled(label, style_dim()));
        }
    }
    f.render_widget(Paragraph::new(Line::from(tab_spans)), chunks[1]);

    // ── rule ──────────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            style_dim(),
        ))),
        chunks[2],
    );

    // ── blank ─────────────────────────────────────────────────────────────
    f.render_widget(Paragraph::new(""), chunks[3]);

    // ── body — scrollable via Paragraph::scroll ───────────────────────────
    render_scrolled_body(f, data, selected, project_filter, exited_hours, chunks[4]);

    // ── help line ─────────────────────────────────────────────────────────
    let exited_hint = match exited_hours {
        None => "[e] show exited".to_string(),
        Some(h) => format!("[e] hide exited  [-/+] {h}h"),
    };
    let help_text =
        format!("[↑↓] move  [←→] tab  [/] search  [↵] attach/spawn  {exited_hint}  [q] quit");

    let help_area = chunks[6];
    if status.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(help_text, style_dim()),
            ])),
            help_area,
        );
    } else {
        // Split help_area into help line + status line.
        let help_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(help_area);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(help_text, style_dim()),
            ])),
            help_chunks[0],
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::raw(status.to_string()),
            ])),
            help_chunks[1],
        );
    }
}

/// Render the scrollable body section into `area`. Builds all content lines,
/// computes scroll offset to keep `selected` in view, then renders via
/// `Paragraph::scroll()`.
pub(super) fn render_scrolled_body(
    f: &mut Frame,
    data: &super::tui_model::TuiData,
    selected: usize,
    project_filter: &str,
    exited_hours: Option<u64>,
    area: Rect,
) {
    use super::tui_model::{filter_live, filter_resumable};
    let fl = filter_live(data, project_filter);
    let fr = filter_resumable(data, project_filter, exited_hours);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut sel_line: Option<usize> = None;

    // Section: Live sessions
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Live sessions", style_bold()),
    ]));
    if fl.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled("(none)", style_dim()),
        ]));
    } else {
        for (i, row) in fl.iter().enumerate() {
            let is_sel = i == selected;
            if is_sel {
                sel_line = Some(lines.len());
            }
            lines.push(live_row_line(row, is_sel));
        }
    }

    // Section: Agents (spawnable)
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Agents", style_bold()),
    ]));
    if data.spawnable.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled("(none)", style_dim()),
        ]));
    } else {
        for (i, row) in data.spawnable.iter().enumerate() {
            let abs_idx = fl.len() + i;
            let is_sel = abs_idx == selected;
            if is_sel {
                sel_line = Some(lines.len());
            }
            lines.push(spawn_row_line(row, is_sel));
        }
    }

    // Section: Exited sessions
    if let Some(hours) = exited_hours {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Exited sessions", style_bold()),
            Span::styled(format!(" (past {hours}h)"), style_dim()),
        ]));
        if fr.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("(none)", style_dim()),
            ]));
        } else {
            for (i, row) in fr.iter().enumerate() {
                let abs_idx = fl.len() + data.spawnable.len() + i;
                let is_sel = abs_idx == selected;
                if is_sel {
                    sel_line = Some(lines.len());
                }
                lines.push(resume_row_line(row, is_sel));
            }
        }
    }

    // Compute scroll offset.
    let viewport = area.height as usize;
    let scroll = compute_scroll(sel_line, viewport, lines.len());

    let para = Paragraph::new(lines)
        .block(Block::default())
        .scroll((scroll as u16, 0));
    f.render_widget(para, area);
}

/// Render the fuzzy project search overlay into a ratatui `Frame`.
pub(super) fn render_search(
    f: &mut Frame,
    pt: &super::tui_model::ProjectTabs,
    query: &str,
    sel: usize,
) {
    use super::tui_model::fuzzy_matches;
    let area = f.area();

    let matches = fuzzy_matches(pt, query);

    // Build match lines.
    let mut body_lines: Vec<Line<'static>> = Vec::new();
    let mut sel_line: Option<usize> = None;
    for (i, proj) in matches.iter().enumerate() {
        let is_sel = i == sel;
        if is_sel {
            sel_line = Some(body_lines.len());
        }
        let cursor = if is_sel { "► " } else { "  " };
        let is_hidden = pt.hidden.contains(proj);
        let line = if is_sel {
            Line::from(vec![
                Span::raw(cursor.to_string()),
                Span::styled(proj.clone(), style_bold()),
            ])
        } else if is_hidden {
            Line::from(vec![
                Span::raw(cursor.to_string()),
                Span::styled(proj.clone(), style_dim()),
            ])
        } else {
            Line::from(vec![Span::raw(cursor.to_string()), Span::raw(proj.clone())])
        };
        body_lines.push(line);
    }
    if matches.is_empty() {
        body_lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled("(no matches)", style_dim()),
        ]));
    }

    // Layout: title(1) + search_input(1) + rule(1) + blank(1) + body(min) + blank(1) + help(1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Length(1), // search input
            Constraint::Length(1), // rule
            Constraint::Length(1), // blank
            Constraint::Min(1),    // matches
            Constraint::Length(1), // blank
            Constraint::Length(1), // help
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled("tenex-edge tmux", style_bold()))),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  / "),
            Span::raw(query.to_string()),
            Span::raw("_"),
        ])),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            style_dim(),
        ))),
        chunks[2],
    );

    f.render_widget(Paragraph::new(""), chunks[3]);

    // Scrollable match list.
    let viewport = chunks[4].height as usize;
    let scroll = compute_scroll(sel_line, viewport, body_lines.len());
    f.render_widget(
        Paragraph::new(body_lines)
            .block(Block::default())
            .scroll((scroll as u16, 0)),
        chunks[4],
    );

    f.render_widget(Paragraph::new(""), chunks[5]);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("[↑↓] move  [↵] select  [esc] cancel", style_dim()),
        ])),
        chunks[6],
    );
}

/// Compute a vertical scroll offset to keep `sel_line` in view within a viewport
/// of `viewport` rows out of `total` content lines.
pub(super) fn compute_scroll(sel_line: Option<usize>, viewport: usize, total: usize) -> usize {
    let mut scroll: usize = 0;
    if let Some(s) = sel_line {
        if s < scroll {
            scroll = s;
        } else if s >= scroll + viewport {
            scroll = s + 1 - viewport;
        }
    }
    let max_scroll = total.saturating_sub(viewport);
    scroll.min(max_scroll)
}
