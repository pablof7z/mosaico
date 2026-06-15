use super::*;

use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame, Terminal,
};

// ── tmux_run ──────────────────────────────────────────────────────────────────

/// Entry point for `tenex-edge tmux <action>`.
pub(super) async fn tmux_run(action: TmuxAction) -> Result<()> {
    match action {
        TmuxAction::Status => tmux_status().await,
        TmuxAction::Send { session } => tmux_send(session).await,
        TmuxAction::Spawn { agent, project } => tmux_spawn(agent, project).await,
        TmuxAction::Attach { session } => tmux_attach(session).await,
        TmuxAction::Resume { session } => tmux_resume(session).await,
        TmuxAction::Sidebar { session, project } => tmux_sidebar(session, project),
    }
}

// ── status ────────────────────────────────────────────────────────────────────

async fn tmux_status() -> Result<()> {
    use owo_colors::OwoColorize as _;

    let v = crate::daemon::blocking::call("tmux_status", serde_json::json!({}))
        .context("tmux_status RPC")?;

    let endpoints = v["endpoints"].as_array().cloned().unwrap_or_default();

    if endpoints.is_empty() {
        println!("No tmux endpoints registered.");
        return Ok(());
    }

    println!(
        "{:<22} {:<8} {:<12} {}",
        "session".bold(),
        "pane".bold(),
        "command".bold(),
        "alive".bold()
    );
    for ep in &endpoints {
        let sid = ep["session_id"].as_str().unwrap_or("");
        let pane = ep["pane_id"].as_str().unwrap_or("");
        let cmd = ep["pane_command"].as_str().unwrap_or("");
        let alive = ep["alive"].as_bool().unwrap_or(false);
        let alive_str = if alive {
            "yes".green().to_string()
        } else {
            "DEAD".red().to_string()
        };
        println!("{sid:<22} {pane:<8} {cmd:<12} {alive_str}");
    }
    Ok(())
}

// ── send (manual doorbell) ────────────────────────────────────────────────────

async fn tmux_send(session: String) -> Result<()> {
    let v = crate::daemon::blocking::call("tmux_send", serde_json::json!({ "session": session }))
        .context("tmux_send RPC")?;

    let injected = v["injected"].as_bool().unwrap_or(false);
    if injected {
        println!("Doorbell injected.");
    } else {
        let reason = v["reason"].as_str().unwrap_or("unknown");
        println!("Doorbell not sent: {reason}");
    }
    Ok(())
}

// ── spawn ─────────────────────────────────────────────────────────────────────

async fn tmux_spawn(agent: String, project: Option<String>) -> Result<()> {
    let project = project
        .unwrap_or_else(|| crate::project::resolve(&std::env::current_dir().unwrap_or_default()));
    let v = crate::daemon::blocking::call(
        "tmux_spawn",
        serde_json::json!({ "agent": agent, "project": project }),
    )
    .context("tmux_spawn RPC")?;

    let pane_id = v["pane_id"].as_str().unwrap_or("?");
    println!("Spawned pane {pane_id} for agent {agent} in project {project}.");
    Ok(())
}

// ── attach ────────────────────────────────────────────────────────────────────

async fn tmux_attach(session: String) -> Result<()> {
    attach_session(&session)
}

// ── resume ────────────────────────────────────────────────────────────────────

async fn tmux_resume(session: String) -> Result<()> {
    let pane = resume_to_pane(&session)?;
    match pane {
        Some(pane_id) => attach_pane(&pane_id),
        None => Ok(()),
    }
}

/// Session id of the currently-selected row IF it is resumable — any local Live
/// row (attachable or not: an in-tmux session can still be replayed) or any
/// Resumable row. `None` for Spawnable rows. The daemon makes the final call on
/// whether a token exists; this just maps cursor → session id.
fn selected_resume_sid(
    live: &[&LiveRow],
    spawnable_count: usize,
    resumable: &[&ResumeRow],
    selected: usize,
) -> Option<String> {
    if selected < live.len() {
        return Some(live[selected].session_id.clone());
    }
    let resume_base = live.len() + spawnable_count;
    if selected >= resume_base {
        return resumable
            .get(selected - resume_base)
            .map(|r| r.session_id.clone());
    }
    None
}

const TWELVE_HOURS: u64 = 12 * 3600;

fn compute_project_tabs(data: &TuiData) -> ProjectTabs {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Count live sessions per project.
    let mut live_count: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for row in &data.live {
        if !row.project.is_empty() {
            *live_count.entry(row.project.clone()).or_insert(0) += 1;
        }
    }
    let live_projects: std::collections::HashSet<String> = live_count.keys().cloned().collect();

    // Track latest created_at per project from resumable sessions.
    let mut last_active: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    for row in &data.resumable {
        if !row.project.is_empty() {
            let e = last_active.entry(row.project.clone()).or_insert(0);
            *e = (*e).max(row.created_at);
        }
    }

    // Projects without live sessions: show if active within 12h, else hide.
    let mut visible_recent: Vec<String> = Vec::new();
    let mut hidden: Vec<String> = Vec::new();

    for (proj, t) in &last_active {
        if live_projects.contains(proj) {
            continue;
        }
        if now.saturating_sub(*t) < TWELVE_HOURS {
            visible_recent.push(proj.clone());
        } else {
            hidden.push(proj.clone());
        }
    }
    visible_recent.sort();
    hidden.sort();

    // Sort live projects by session count descending, then alphabetically.
    let mut live_sorted: Vec<String> = live_projects.into_iter().collect();
    live_sorted.sort_by(|a, b| {
        let ca = live_count.get(a).copied().unwrap_or(0);
        let cb = live_count.get(b).copied().unwrap_or(0);
        cb.cmp(&ca).then(a.cmp(b))
    });

    let mut visible: Vec<String> = live_sorted;
    visible.extend(visible_recent);

    ProjectTabs { visible, hidden }
}

fn tab_project<'a>(tabs: &'a [String], tab_idx: usize) -> Option<&'a str> {
    if tab_idx == 0 {
        None
    } else {
        tabs.get(tab_idx - 1).map(|s| s.as_str())
    }
}

fn filter_live<'a>(data: &'a TuiData, project_filter: Option<&str>) -> Vec<&'a LiveRow> {
    data.live
        .iter()
        .filter(|r| project_filter.map(|p| r.project == p).unwrap_or(true))
        .collect()
}

fn filter_resumable<'a>(
    data: &'a TuiData,
    project_filter: Option<&str>,
    exited_hours: Option<u64>,
) -> Vec<&'a ResumeRow> {
    let hours = match exited_hours {
        None => return vec![],
        Some(h) => h,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cutoff = now.saturating_sub(hours * 3600);
    data.resumable
        .iter()
        .filter(|r| {
            r.created_at >= cutoff
                && project_filter.map(|p| r.project == p).unwrap_or(true)
        })
        .collect()
}

fn update_tabs_after_refresh(data: &TuiData, pt: &mut ProjectTabs, tab_idx: &mut usize) {
    let mut new_pt = compute_project_tabs(data);
    // Preserve the currently-selected project tab even if it became "hidden"
    // (e.g., selected via fuzzy search but older than 7 days).
    let current_proj = if *tab_idx > 0 {
        pt.visible.get(*tab_idx - 1).cloned()
    } else {
        None
    };
    if let Some(proj) = current_proj {
        if let Some(idx) = new_pt.visible.iter().position(|p| *p == proj) {
            *tab_idx = idx + 1;
        } else if let Some(hi) = new_pt.hidden.iter().position(|p| *p == proj) {
            // Was hidden but user has it selected — keep it visible.
            let pinned = new_pt.hidden.remove(hi);
            new_pt.visible.push(pinned);
            *tab_idx = new_pt.visible.len();
        } else {
            *tab_idx = 0;
        }
    }
    *pt = new_pt;
}

/// Compute fuzzy matches for `query` across all projects (visible + hidden).
/// Case-insensitive substring match; visible projects listed first.
fn fuzzy_matches(pt: &ProjectTabs, query: &str) -> Vec<String> {
    let q = query.to_lowercase();
    pt.visible
        .iter()
        .chain(pt.hidden.iter())
        .filter(|p| p.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

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
    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
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
fn live_row_line(row: &LiveRow, is_sel: bool, project_filter: Option<&str>) -> Line<'static> {
    let cursor = if is_sel { "► " } else { "  " };
    let label = if project_filter.is_none() {
        format!("{}@{}", row.slug, row.project)
    } else {
        format!("{}@{}", row.slug, row.host)
    };
    let session_tag = format!(" [session {}]", row.session_short);
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
fn spawn_row_line(row: &SpawnRow, is_sel: bool) -> Line<'static> {
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
fn resume_row_line(row: &ResumeRow, is_sel: bool, project_filter: Option<&str>) -> Line<'static> {
    let cursor = if is_sel { "► " } else { "  " };
    let label = if project_filter.is_none() {
        format!("{}@{}", row.slug, row.project)
    } else {
        row.slug.clone()
    };
    let session_tag = format!(" [session {}]", row.session_short);
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
fn render_main(
    f: &mut Frame,
    data: &TuiData,
    selected: usize,
    status: &str,
    tabs: &[String],
    tab_idx: usize,
    exited_hours: Option<u64>,
) {
    let area = f.area();

    let project_filter = tab_project(tabs, tab_idx);

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
    let title_line = Line::from(vec![Span::styled(
        "tenex-edge tmux",
        style_bold(),
    )]);
    f.render_widget(Paragraph::new(title_line), chunks[0]);

    // ── tab bar ───────────────────────────────────────────────────────────
    let mut tab_spans: Vec<Span> = vec![Span::raw("  ")];
    if tab_idx == 0 {
        tab_spans.push(Span::styled("[All]", style_bold()));
    } else {
        tab_spans.push(Span::styled("[All]", style_dim()));
    }
    for (i, tab) in tabs.iter().enumerate() {
        tab_spans.push(Span::raw(" "));
        let label = format!("[{tab}]");
        if tab_idx == i + 1 {
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
    render_scrolled_body(
        f,
        data,
        selected,
        project_filter,
        exited_hours,
        chunks[4],
    );

    // ── help line ─────────────────────────────────────────────────────────
    let exited_hint = match exited_hours {
        None => "[e] show exited".to_string(),
        Some(h) => format!("[e] hide exited  [-/+] {h}h"),
    };
    let help_text = format!(
        "[↑↓] move  [←→] tab  [/] search  [↵] attach/spawn  {exited_hint}  [q] quit"
    );

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
fn render_scrolled_body(
    f: &mut Frame,
    data: &TuiData,
    selected: usize,
    project_filter: Option<&str>,
    exited_hours: Option<u64>,
    area: Rect,
) {
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
            lines.push(live_row_line(row, is_sel, project_filter));
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
                lines.push(resume_row_line(row, is_sel, project_filter));
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
fn render_search(f: &mut Frame, pt: &ProjectTabs, query: &str, sel: usize) {
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
            Line::from(vec![
                Span::raw(cursor.to_string()),
                Span::raw(proj.clone()),
            ])
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
        Paragraph::new(Line::from(Span::styled(
            "tenex-edge tmux",
            style_bold(),
        ))),
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
            Span::styled(
                "[↑↓] move  [↵] select  [esc] cancel",
                style_dim(),
            ),
        ])),
        chunks[6],
    );
}

/// Compute a vertical scroll offset to keep `sel_line` in view within a viewport
/// of `viewport` rows out of `total` content lines.
fn compute_scroll(sel_line: Option<usize>, viewport: usize, total: usize) -> usize {
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

/// TUI variant: resume `session`, returning the new pane id or an `Err(message)`
/// suitable for the status line (never writes to stderr, which raw mode mangles).
fn resume_in_tui(session: &str) -> std::result::Result<String, String> {
    let v = crate::daemon::blocking::call("tmux_resume", serde_json::json!({ "session": session }))
        .map_err(|e| format!("Resume failed: {e}"))?;
    match v["pane_id"].as_str() {
        Some(p) => Ok(p.to_string()),
        None => Err(format!(
            "Cannot resume: {}",
            v["error"].as_str().unwrap_or("unknown error")
        )),
    }
}

/// Ask the daemon to resume `session`, returning the new pane id (or `None`,
/// after printing the error). Shared by the CLI verb and the TUI.
fn resume_to_pane(session: &str) -> Result<Option<String>> {
    let v =
        crate::daemon::blocking::call("tmux_resume", serde_json::json!({ "session": session }))
            .context("tmux_resume RPC")?;
    match v["pane_id"].as_str() {
        Some(p) => Ok(Some(p.to_string())),
        None => {
            let err = v["error"].as_str().unwrap_or("unknown error");
            eprintln!("Cannot resume: {err}");
            Ok(None)
        }
    }
}

// ── shared attach logic ───────────────────────────────────────────────────────

/// Resolve a session id to its live tmux pane id via the daemon, or `None`.
fn pane_for_session(session_id: &str) -> Option<String> {
    let v = crate::daemon::blocking::call("tmux_attach", serde_json::json!({ "session": session_id }))
        .ok()?;
    v["pane_id"].as_str().map(str::to_string)
}

fn attach_session(session_id: &str) -> Result<()> {
    let v =
        crate::daemon::blocking::call("tmux_attach", serde_json::json!({ "session": session_id }))
            .context("tmux_attach RPC")?;

    let pane_id = match v["pane_id"].as_str() {
        Some(p) => p.to_string(),
        None => {
            let err = v["error"].as_str().unwrap_or("unknown error");
            eprintln!("Cannot attach: {err}");
            return Ok(());
        }
    };

    attach_pane(&pane_id)
}

// ── TUI ───────────────────────────────────────────────────────────────────────

/// RAII guard for raw mode + alternate screen. Used to suspend/resume when
/// handing off the tty to a `tmux attach-session` child.
struct TuiTerminal;

impl TuiTerminal {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }

    /// Temporarily restore the normal terminal so a child process (e.g. a tmux
    /// client) can own the tty, without dropping our guard. Pair with `resume`.
    fn suspend() {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }

    /// Re-enter the alternate-screen raw-mode TUI after a `suspend`.
    fn resume() {
        let _ = terminal::enable_raw_mode();
        let _ = execute!(io::stdout(), EnterAlternateScreen, Hide);
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}

struct LiveRow {
    slug: String,
    host: String,
    project: String,
    session_id: String,    // full raw id for RPC calls
    session_short: String, // short display code (6 chars)
    status: String,
    attachable: bool, // has a live tmux endpoint
}

struct SpawnRow {
    slug: String,
    host: String,
    command: String,
}

struct ResumeRow {
    slug: String,
    project: String,
    session_id: String,    // full raw id for RPC calls
    session_short: String, // short display code (6 chars)
    title: String,
    created_at: u64,
}

/// Tabs computed from live data: visible projects ordered by activity (live
/// first, then recently-active), plus hidden projects (>7 days inactive).
struct ProjectTabs {
    /// Projects shown in the tab bar. Order: projects with live sessions first
    /// (alphabetically), then recently-active projects (alphabetically).
    visible: Vec<String>,
    /// Projects with no activity in the past 7 days. Only reachable via search.
    hidden: Vec<String>,
}

impl PartialEq for ProjectTabs {
    fn eq(&self, other: &Self) -> bool {
        self.visible == other.visible && self.hidden == other.hidden
    }
}

enum TuiMode {
    Normal,
    Search { query: String, sel: usize },
}

struct TuiData {
    live: Vec<LiveRow>,
    spawnable: Vec<SpawnRow>,
    resumable: Vec<ResumeRow>,
}

fn fetch_tui_data() -> Result<TuiData> {
    let v = crate::daemon::blocking::call(
        "who",
        serde_json::json!({
            "project": null,
            "all": false,
            "all_projects": true,
            "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        }),
    )?;

    let live = v["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|r| !r["remote"].as_bool().unwrap_or(false))
        .map(|r| {
            let raw_id = r["session_id"].as_str().unwrap_or("").to_string();
            let session_short = SessionId::from(raw_id.as_str()).to_string();
            LiveRow {
                slug: r["slug"].as_str().unwrap_or("").to_string(),
                host: r["host"].as_str().unwrap_or("").to_string(),
                project: r["project"].as_str().unwrap_or("").to_string(),
                session_id: raw_id,
                session_short,
                status: r["status"].as_str().unwrap_or("").to_string(),
                attachable: r["attachable"].as_bool().unwrap_or(false),
            }
        })
        .collect();

    let spawnable = v["spawnable"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|r| SpawnRow {
            slug: r["slug"].as_str().unwrap_or("").to_string(),
            host: r["host"].as_str().unwrap_or("").to_string(),
            command: r["command"].as_str().unwrap_or("").to_string(),
        })
        .collect();

    // Resumable (dead, but replayable) sessions come from a dedicated RPC.
    // Fail soft: an older daemon without it just yields an empty section.
    let resumable = crate::daemon::blocking::call("tmux_resumable", serde_json::json!({}))
        .ok()
        .and_then(|rv| rv["resumable"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|r| {
            let raw_id = r["session_id"].as_str().unwrap_or("").to_string();
            let session_short = SessionId::from(raw_id.as_str()).to_string();
            ResumeRow {
                slug: r["slug"].as_str().unwrap_or("").to_string(),
                project: r["project"].as_str().unwrap_or("").to_string(),
                session_id: raw_id,
                session_short,
                title: r["title"].as_str().unwrap_or("").to_string(),
                created_at: r["created_at"].as_u64().unwrap_or(0),
            }
        })
        .collect();

    Ok(TuiData {
        live,
        spawnable,
        resumable,
    })
}

/// Interactive TUI for `tenex-edge tmux` (bare, no subcommand).
/// Shows live sessions and spawnable agents; lets the user attach or spawn.
///
/// When `popup` is true the TUI is running inside a `display-popup` overlay:
/// instead of attaching inline (which would trap the session inside the popup),
/// selecting a session switches the underlying client and exits, closing the
/// popup so the chosen session takes over the full client.
pub(super) fn tmux_tui(popup: bool) -> Result<()> {
    let refresh = Duration::from_secs(2);
    let mut selected: usize = 0;
    let mut status_msg = String::new();
    let mut tab_idx: usize = 0;
    let mut show_exited: bool = false;
    let mut exited_hours: u64 = 4;
    let mut mode = TuiMode::Normal;

    // Initial fetch before entering raw mode: fail fast if daemon is down.
    let mut data = fetch_tui_data()?;
    let mut pt = compute_project_tabs(&data);

    {
        let _terminal = TuiTerminal::enter()?;
        // Create ratatui terminal on top of the crossterm alternate screen
        // already enabled by TuiTerminal::enter().
        let mut ratatui_term =
            Terminal::new(CrosstermBackend::new(io::stdout()))?;

        let mut next_refresh = Instant::now() + refresh;

        loop {
            // ── draw ──────────────────────────────────────────────────────
            match &mode {
                TuiMode::Search { query, sel } => {
                    let q = query.clone();
                    let s = *sel;
                    ratatui_term.draw(|f| render_search(f, &pt, &q, s))?;
                }
                TuiMode::Normal => {
                    let exited_opt = if show_exited { Some(exited_hours) } else { None };
                    // Compute filtered totals (borrows released at end of block).
                    let total = {
                        let pf = tab_project(&pt.visible, tab_idx);
                        filter_live(&data, pf).len()
                            + data.spawnable.len()
                            + filter_resumable(&data, pf, exited_opt).len()
                    };
                    if total > 0 && selected >= total {
                        selected = total - 1;
                    }
                    let tabs_snap = pt.visible.clone();
                    let status_snap = status_msg.clone();
                    ratatui_term.draw(|f| {
                        render_main(
                            f,
                            &data,
                            selected,
                            &status_snap,
                            &tabs_snap,
                            tab_idx,
                            exited_opt,
                        )
                    })?;
                }
            }

            let wait = next_refresh
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));

            let mut should_break = false;
            let mut pending_attach: Option<String> = None;

            if event::poll(wait)? {
                if let TermEvent::Key(key) = event::read()? {
                    match &mut mode {
                        // ── search mode ───────────────────────────────────
                        TuiMode::Search { query, sel } => {
                            match key.code {
                                KeyCode::Esc => {
                                    mode = TuiMode::Normal;
                                }
                                KeyCode::Enter => {
                                    let matches = fuzzy_matches(&pt, query);
                                    if let Some(proj) = matches.get(*sel).cloned() {
                                        if let Some(idx) =
                                            pt.visible.iter().position(|p| *p == proj)
                                        {
                                            tab_idx = idx + 1;
                                        } else {
                                            // Hidden project: inject into visible temporarily.
                                            pt.hidden.retain(|p| p != &proj);
                                            pt.visible.push(proj);
                                            tab_idx = pt.visible.len();
                                        }
                                        selected = 0;
                                    }
                                    mode = TuiMode::Normal;
                                    status_msg.clear();
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    *sel = sel.saturating_sub(1);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    let n = fuzzy_matches(&pt, query).len();
                                    if *sel + 1 < n {
                                        *sel += 1;
                                    }
                                }
                                KeyCode::Backspace => {
                                    query.pop();
                                    *sel = 0;
                                }
                                KeyCode::Char(c)
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    query.push(c);
                                    *sel = 0;
                                }
                                _ => {}
                            }
                        }
                        // ── normal mode ───────────────────────────────────
                        TuiMode::Normal => {
                            let exited_opt = if show_exited { Some(exited_hours) } else { None };
                            // We need filtered views. Use a block so borrows of
                            // `data` are released before any `data = fresh` below.
                            let total = {
                                let pf = tab_project(&pt.visible, tab_idx);
                                filter_live(&data, pf).len()
                                    + data.spawnable.len()
                                    + filter_resumable(&data, pf, exited_opt).len()
                            };
                            {
                                let pf = tab_project(&pt.visible, tab_idx);
                                let fl = filter_live(&data, pf);
                                let fr = filter_resumable(&data, pf, exited_opt);

                                match key.code {
                                    KeyCode::Char('q') | KeyCode::Esc => {
                                        should_break = true;
                                    }
                                    KeyCode::Char('c')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        should_break = true;
                                    }
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        selected = selected.saturating_sub(1);
                                        status_msg.clear();
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        if total > 0 && selected + 1 < total {
                                            selected += 1;
                                        }
                                        status_msg.clear();
                                    }
                                    // Left/right: switch project tabs.
                                    KeyCode::Left => {
                                        if tab_idx > 0 {
                                            tab_idx -= 1;
                                            selected = 0;
                                            status_msg.clear();
                                        }
                                    }
                                    KeyCode::Right => {
                                        if tab_idx < pt.visible.len() {
                                            tab_idx += 1;
                                            selected = 0;
                                            status_msg.clear();
                                        }
                                    }
                                    // /: enter fuzzy project search.
                                    KeyCode::Char('/') => {
                                        mode = TuiMode::Search {
                                            query: String::new(),
                                            sel: 0,
                                        };
                                    }
                                    // e: toggle exited sessions.
                                    KeyCode::Char('e') => {
                                        show_exited = !show_exited;
                                        status_msg.clear();
                                    }
                                    // +/= / -: adjust the hours window (only when exited is shown).
                                    KeyCode::Char('+') | KeyCode::Char('=') if show_exited => {
                                        exited_hours = match exited_hours {
                                            h if h >= 48 => h + 24,
                                            h if h >= 12 => h + 6,
                                            h => h + 1,
                                        };
                                        status_msg.clear();
                                    }
                                    KeyCode::Char('-') if show_exited => {
                                        exited_hours = match exited_hours {
                                            h if h > 48 => h - 24,
                                            h if h > 12 => h - 6,
                                            h => h.saturating_sub(1).max(1),
                                        };
                                        status_msg.clear();
                                    }
                                    KeyCode::Enter | KeyCode::Char('a') => {
                                        if selected < fl.len() && fl[selected].attachable {
                                            match pane_for_session(&fl[selected].session_id) {
                                                Some(p) => pending_attach = Some(p),
                                                None => {
                                                    status_msg =
                                                        "Session pane not found.".to_string()
                                                }
                                            }
                                        } else {
                                            let si = selected.saturating_sub(fl.len());
                                            if selected >= fl.len() && si < data.spawnable.len() {
                                                let slug = data.spawnable[si].slug.clone();
                                                // Spawn into the selected project tab's dir, not
                                                // the TUI process's cwd. Fall back to cwd-resolution
                                                // only on the "all projects" tab (no filter).
                                                let project = pf.map(str::to_string).unwrap_or_else(
                                                    || {
                                                        crate::project::resolve(
                                                            &std::env::current_dir()
                                                                .unwrap_or_default(),
                                                        )
                                                    },
                                                );
                                                status_msg = format!("Spawning {slug}...");
                                                // Render the status immediately before blocking.
                                                let tabs_snap = pt.visible.clone();
                                                let status_snap = status_msg.clone();
                                                let _ = ratatui_term.draw(|f| {
                                                    render_main(
                                                        f,
                                                        &data,
                                                        selected,
                                                        &status_snap,
                                                        &tabs_snap,
                                                        tab_idx,
                                                        exited_opt,
                                                    )
                                                });
                                                match crate::daemon::blocking::call(
                                                    "tmux_spawn",
                                                    serde_json::json!({
                                                        "agent": slug,
                                                        "project": project,
                                                    }),
                                                ) {
                                                    Ok(v) => {
                                                        pending_attach =
                                                            v["pane_id"].as_str().map(str::to_string);
                                                    }
                                                    Err(e) => {
                                                        status_msg = format!("Spawn failed: {e}")
                                                    }
                                                }
                                            } else {
                                                match selected_resume_sid(
                                                    &fl,
                                                    data.spawnable.len(),
                                                    &fr,
                                                    selected,
                                                ) {
                                                    Some(sid) => {
                                                        status_msg = "Resuming...".to_string();
                                                        // Render the status immediately before blocking.
                                                        let tabs_snap = pt.visible.clone();
                                                        let status_snap = status_msg.clone();
                                                        let _ = ratatui_term.draw(|f| {
                                                            render_main(
                                                                f,
                                                                &data,
                                                                selected,
                                                                &status_snap,
                                                                &tabs_snap,
                                                                tab_idx,
                                                                exited_opt,
                                                            )
                                                        });
                                                        match resume_in_tui(&sid) {
                                                            Ok(pane) => pending_attach = Some(pane),
                                                            Err(msg) => status_msg = msg,
                                                        }
                                                    }
                                                    None => {}
                                                }
                                            }
                                        }
                                    }
                                    KeyCode::Char('r') => {
                                        if let Some(sid) = selected_resume_sid(
                                            &fl,
                                            data.spawnable.len(),
                                            &fr,
                                            selected,
                                        ) {
                                            status_msg = "Resuming...".to_string();
                                            // Render the status immediately before blocking.
                                            let tabs_snap = pt.visible.clone();
                                            let status_snap = status_msg.clone();
                                            let _ = ratatui_term.draw(|f| {
                                                render_main(
                                                    f,
                                                    &data,
                                                    selected,
                                                    &status_snap,
                                                    &tabs_snap,
                                                    tab_idx,
                                                    exited_opt,
                                                )
                                            });
                                            match resume_in_tui(&sid) {
                                                Ok(pane) => pending_attach = Some(pane),
                                                Err(msg) => status_msg = msg,
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                                // fl, fr, pf borrows of `data` are released here.
                            }
                        }
                    }
                }
            }

            if should_break {
                break;
            }

            // Attach. In popup mode, switch the underlying client to the chosen
            // session and exit so the `display-popup` closes and the session
            // takes over the full client. Otherwise attach inline (blocking),
            // then return to the TUI.
            if let Some(pane) = pending_attach {
                if popup {
                    if let Some(session) = session_of_pane(&pane) {
                        ensure_sidebar(&session);
                        bind_sidebar_keys();
                        let _ = std::process::Command::new("tmux")
                            .args(["switch-client", "-t", &session])
                            .status();
                    }
                    break; // exit the TUI → display-popup -E closes
                }
                // Suspend ratatui/crossterm so the tmux client owns the tty.
                TuiTerminal::suspend();
                let res = attach_pane_blocking(&pane);
                TuiTerminal::resume();
                // ratatui needs a full redraw after the terminal is restored.
                ratatui_term.clear()?;
                status_msg = match res {
                    Ok(()) => String::new(),
                    Err(e) => format!("Attach failed: {e:#}"),
                };
                if let Ok(fresh) = fetch_tui_data() {
                    update_tabs_after_refresh(&fresh, &mut pt, &mut tab_idx);
                    data = fresh;
                }
                next_refresh = Instant::now() + refresh;
            }

            // Periodic refresh.
            if Instant::now() >= next_refresh {
                if let Ok(fresh) = fetch_tui_data() {
                    update_tabs_after_refresh(&fresh, &mut pt, &mut tab_idx);
                    data = fresh;
                }
                next_refresh = Instant::now() + refresh;
            }
        }
    }; // _terminal dropped here — raw mode disabled, alternate screen exited

    Ok(())
}

/// Resolve a pane id (e.g. "%7") to `(session_name, window_index)` by scanning
/// every pane in every session. Returns `None` if the pane is gone.
fn resolve_pane_location(pane_id: &str) -> Option<(String, String)> {
    let out = std::process::Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_id} #{session_name} #{window_index}",
        ])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().find_map(|line| {
        let mut parts = line.splitn(3, ' ');
        let pid = parts.next()?;
        let session = parts.next()?;
        let window = parts.next()?;
        if pid == pane_id {
            Some((session.to_string(), window.to_string()))
        } else {
            None
        }
    })
}

/// The tmux session that owns `pane_id` (one session per agent now), or `None`
/// if the pane is gone.
fn session_of_pane(pane_id: &str) -> Option<String> {
    resolve_pane_location(pane_id).map(|(session, _window)| session)
}

/// Attach to the session owning `pane_id` as a BLOCKING child, returning when the
/// user detaches (Ctrl-b d) or the session ends. `$TMUX` is stripped from the
/// child so it works even when the caller is itself inside tmux (nested attach) —
/// this is what lets the `tenex-edge tmux` TUI stay running underneath and be
/// returned to afterward. No grouped "view" session is needed: each agent is its
/// own single-window session, so there is no current-window pointer to mirror.
fn attach_pane_blocking(pane_id: &str) -> Result<()> {
    let Some(session) = session_of_pane(pane_id) else {
        anyhow::bail!("pane {pane_id} not found in any tmux session");
    };
    // Ensure this session has a sidebar pane before we enter it.
    ensure_sidebar(&session);
    bind_sidebar_keys();
    std::process::Command::new("tmux")
        .args(["attach-session", "-t", &session])
        .env_remove("TMUX")
        .status()
        .context("tmux attach-session")?;
    Ok(())
}

/// Attach by replacing this process (for the one-shot CLI verbs, where returning
/// to a shell on detach is the right behavior). Inside tmux it switches the
/// current client; outside it execs `attach-session`.
fn attach_pane(pane_id: &str) -> Result<()> {
    let Some(session) = session_of_pane(pane_id) else {
        eprintln!("Pane {pane_id} not found in any tmux session.");
        return Ok(());
    };
    // Ensure this session has a sidebar pane before we enter it.
    ensure_sidebar(&session);
    bind_sidebar_keys();

    let in_tmux = std::env::var("TMUX").map(|v| !v.is_empty()).unwrap_or(false);
    if in_tmux {
        let status = std::process::Command::new("tmux")
            .args(["switch-client", "-t", &session])
            .status()
            .context("tmux switch-client")?;
        if !status.success() {
            eprintln!("tmux switch-client failed for session {session}");
        }
        return Ok(());
    }

    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new("tmux")
        .args(["attach-session", "-t", &session])
        .exec(); // replaces this process; only returns on error
    anyhow::bail!("exec tmux attach-session: {err}");
}

// ── sidebar ───────────────────────────────────────────────────────────────────

/// Fixed sidebar width in columns. tmux rescales panes proportionally when the
/// client resizes, so a one-time `-l` at split time isn't enough — we also pin
/// it with a `client-resized` hook (see `ensure_sidebar`).
const SIDEBAR_COLS: &str = "40";

/// Idempotently inject a sidebar pane into `session` if one does not already
/// exist. The sidebar is a narrow left pane (a fixed `SIDEBAR_COLS` wide)
/// running `tenex-edge tmux sidebar --session <session>`. Focus stays on the
/// agent pane.
///
/// Errors are swallowed (eprintln at most) — the caller is about to hand off
/// the terminal and does not need sidebar injection to be fatal.
fn ensure_sidebar(session: &str) {
    // List all panes in the session and their start commands.
    let out = std::process::Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            session,
            "-F",
            "#{pane_id} #{pane_start_command}",
        ])
        .output();

    match out {
        Err(e) => {
            eprintln!("ensure_sidebar: list-panes failed: {e}");
            return;
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // If any pane is already running our sidebar, do nothing.
            if stdout.lines().any(|line| line.contains("tmux sidebar")) {
                return;
            }
        }
    }

    // Split a fixed-width pane on the LEFT (-b), keep the current pane focused
    // (-d). Capture the new pane id so we can pin its width on resize.
    let sidebar_cmd = format!("tenex-edge tmux sidebar --session {session}");
    let out = std::process::Command::new("tmux")
        .args([
            "split-window",
            "-h",  // horizontal split (side-by-side)
            "-b",  // new pane goes to the LEFT of the current pane
            "-l",
            SIDEBAR_COLS,
            "-d",  // don't switch focus to the new pane
            "-P",  // print the new pane id
            "-F",
            "#{pane_id}",
            "-t",
            session,
            &sidebar_cmd,
        ])
        .output();

    let pane_id = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Ok(_) => {
            eprintln!("ensure_sidebar: split-window exited non-zero for session {session}");
            return;
        }
        Err(e) => {
            eprintln!("ensure_sidebar: split-window failed: {e}");
            return;
        }
    };

    // Pin the sidebar width: when the client resizes, tmux would otherwise
    // rescale this pane proportionally. A `client-resized` hook re-fixes it.
    // (resize-pane fires `window-layout-changed`, not `client-resized`, so this
    // does not recurse.) The hook is session-scoped and dies with the session.
    let resize_cmd = format!("resize-pane -t {pane_id} -x {SIDEBAR_COLS}");
    let _ = std::process::Command::new("tmux")
        .args(["set-hook", "-t", session, "client-resized", &resize_cmd])
        .status();
}

/// Bind Alt-s (focus sidebar/left), Alt-a (focus agent/right), and Alt-t (open
/// the session-switcher popup) in the root key table. Re-binding is idempotent,
/// so this is safe to call every attach.
fn bind_sidebar_keys() {
    let _ = std::process::Command::new("tmux")
        .args(["bind-key", "-T", "root", "M-s", "select-pane", "-L"])
        .status();
    let _ = std::process::Command::new("tmux")
        .args(["bind-key", "-T", "root", "M-a", "select-pane", "-R"])
        .status();
    // Alt-t: full-screen quick-switcher in a popup. `--popup` makes the TUI
    // switch the underlying client and exit on selection (instead of attaching
    // inline inside the popup); `-E` closes the popup when it exits.
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "root",
            "M-t",
            "display-popup",
            "-E",
            "-w",
            "80%",
            "-h",
            "80%",
            "tenex-edge tmux --popup",
        ])
        .status();
}

/// Resolve the current tmux session name from within a running tmux client.
/// Returns `None` if not inside tmux or the command fails.
fn current_tmux_session() -> Option<String> {
    let out = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#{client_session}"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// The sidebar TUI: a long-running narrow-pane process that lists project
/// sessions and lets the user switch between them with arrow keys + Enter.
///
/// This is the implementation of `tenex-edge tmux sidebar`.
fn tmux_sidebar(session_arg: Option<String>, project_arg: Option<String>) -> Result<()> {
    // Resolve the session this sidebar belongs to.
    let my_session = session_arg
        .or_else(current_tmux_session)
        .unwrap_or_default();

    let refresh = Duration::from_secs(2);
    let mut selected: usize = 0;
    let mut project_filter: Option<String> = project_arg;

    // Fetch initial data — fail fast if daemon is down.
    let mut data = fetch_tui_data()?;

    // Derive project from our session if no --project was given.
    if project_filter.is_none() {
        if let Some(row) = data.live.iter().find(|r| {
            r.session_id == my_session
                || session_name_for_id(&r.session_id).as_deref() == Some(my_session.as_str())
        }) {
            project_filter = Some(row.project.clone());
        }
    }

    // Pre-select our own session in the list.
    let rows_for_project = |live: &[LiveRow], pf: &Option<String>| -> Vec<usize> {
        live.iter()
            .enumerate()
            .filter(|(_, r)| {
                pf.as_deref()
                    .map(|p| r.project == p)
                    .unwrap_or(true)
            })
            .map(|(i, _)| i)
            .collect()
    };

    {
        let _terminal = TuiTerminal::enter()?;
        let mut ratatui_term =
            Terminal::new(CrosstermBackend::new(io::stdout())).context("init sidebar terminal")?;
        let mut next_refresh = Instant::now() + refresh;

        loop {
            let indices = rows_for_project(&data.live, &project_filter);
            let total = indices.len();

            if total > 0 && selected >= total {
                selected = total.saturating_sub(1);
            }

            // Render the sidebar.
            let proj_snap = project_filter.clone().unwrap_or_default();
            ratatui_term.draw(|f| {
                sidebar_render(f, &data.live, &indices, selected, &my_session, &proj_snap)
            })?;

            let wait = next_refresh
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));

            if event::poll(wait)? {
                if let TermEvent::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            break
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            selected = selected.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if total > 0 && selected + 1 < total {
                                selected += 1;
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(&abs_idx) = indices.get(selected) {
                                let target_row = &data.live[abs_idx];
                                let target_sid = target_row.session_id.clone();

                                // Don't switch if we're already here.
                                let target_name =
                                    session_name_for_id(&target_sid).unwrap_or_default();
                                if target_name == my_session {
                                    continue;
                                }

                                // Ensure the target session has its own sidebar.
                                ensure_sidebar(&target_name);

                                // Switch the tmux client to the target session.
                                let _ = std::process::Command::new("tmux")
                                    .args(["switch-client", "-t", &target_name])
                                    .status();
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Periodic refresh.
            if Instant::now() >= next_refresh {
                if let Ok(fresh) = fetch_tui_data() {
                    data = fresh;
                    // Re-derive project filter from updated data if still unset.
                    if project_filter.is_none() {
                        if let Some(row) = data.live.iter().find(|r| {
                            session_name_for_id(&r.session_id).as_deref()
                                == Some(my_session.as_str())
                        }) {
                            project_filter = Some(row.project.clone());
                        }
                    }
                }
                next_refresh = Instant::now() + refresh;
            }
        }
    }

    Ok(())
}

/// Render the sidebar contents into a narrow pane via ratatui.
fn sidebar_render(
    f: &mut Frame,
    live: &[LiveRow],
    indices: &[usize],
    selected: usize,
    my_session: &str,
    project: &str,
) {
    let area = f.area();
    let w = area.width as usize;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Length(1), // rule
            Constraint::Min(1),    // session list
            Constraint::Length(1), // help
        ])
        .split(area);

    // ── title: "sessions" bold + project dim ──────────────────────────────
    let mut title_spans = vec![Span::styled("sessions", style_bold())];
    if !project.is_empty() {
        title_spans.push(Span::styled(format!(" {project}"), style_dim()));
    }
    f.render_widget(Paragraph::new(Line::from(title_spans)), chunks[0]);

    // ── rule ──────────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("─".repeat(w), style_dim()))),
        chunks[1],
    );

    // ── session list ──────────────────────────────────────────────────────
    let mut lines: Vec<Line> = Vec::new();
    if indices.is_empty() {
        lines.push(Line::from(Span::styled("  (no sessions)", style_dim())));
    } else {
        for (sel_idx, &abs_idx) in indices.iter().enumerate() {
            let row = &live[abs_idx];
            let is_current = session_name_for_id(&row.session_id).as_deref() == Some(my_session)
                || row.session_id == my_session;
            let is_sel = sel_idx == selected;
            let cursor = if is_sel { "►" } else { " " };

            // Compact status badge (max 4 chars).
            let status_badge = if row.status.trim().is_empty() {
                "idle".to_string()
            } else {
                let s = row.status.trim();
                s.chars().take(4).collect()
            };

            // Truncate slug to fit: cursor(1) + space + slug + space + badge.
            let slug_max = w.saturating_sub(1 + 1 + status_badge.len() + 1);
            let slug_display: String = row.slug.chars().take(slug_max.max(1)).collect();

            let (cursor_style, slug_style) = match (is_current, is_sel) {
                (true, true) => (style_cyan_bold(), style_cyan_bold()),
                (true, false) => (Style::default(), style_cyan()),
                (false, true) => (style_bold(), style_bold()),
                (false, false) => (Style::default(), style_dim()),
            };

            lines.push(Line::from(vec![
                Span::styled(cursor.to_string(), cursor_style),
                Span::raw(" "),
                Span::styled(slug_display, slug_style),
                Span::raw(" "),
                Span::styled(status_badge, style_dim()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), chunks[2]);

    // ── help ──────────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("↑↓ move  ↵ switch", style_dim()))),
        chunks[3],
    );
}

/// Look up the tmux session NAME for a given tenex-edge session_id by scanning
/// all live sessions. Returns `None` if the id isn't registered in tmux.
fn session_name_for_id(session_id: &str) -> Option<String> {
    // Ask the daemon for the pane registered to this session_id.
    let pane = pane_for_session(session_id)?;
    session_of_pane(&pane)
}
