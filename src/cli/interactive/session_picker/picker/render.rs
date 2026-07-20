use super::{delete::PendingDelete, PickerState};
use crate::cli::{agents::AgentKind, interactive::session_picker::HomeChoice};
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
const WARNING: Color = Color::Indexed(214);

pub(super) fn draw(frame: &mut Frame<'_>, state: &PickerState) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    if area.height < 3 {
        frame.render_widget(Paragraph::new("Mosaico"), area);
        return;
    }
    if let Some(projects) = state.project_picker.as_ref() {
        draw_projects(frame, projects);
        return;
    }

    let [title_area, options_area, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let (sessions, agents) = state.counts();
    let query = if state.filtering && state.query.is_empty() {
        Span::styled("type to search", Style::default().fg(MUTED))
    } else if state.filtering {
        Span::styled(state.query.as_str(), Style::default().fg(ACCENT))
    } else {
        Span::styled("press /", Style::default().fg(MUTED))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Mosaico", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("  {sessions} sessions · {agents} agents"),
                Style::default().fg(MUTED),
            ),
            Span::raw("  History: "),
            Span::styled(state.range.label(), Style::default().fg(ACCENT)),
            Span::raw("  Project: "),
            Span::styled(state.project_label(), Style::default().fg(ACCENT)),
            Span::raw("  Search: "),
            query,
        ])),
        title_area,
    );

    let width = usize::from(options_area.width.saturating_sub(4));
    let now = crate::util::now_secs();
    let items = state
        .window(usize::from(options_area.height))
        .into_iter()
        .map(|entry| {
            let focused = entry.position == state.cursor;
            let mut lines = Vec::with_capacity(3);
            if let Some(header) = entry.header {
                let count = if entry.choice.is_session() {
                    sessions
                } else {
                    agents
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        header,
                        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  {count}"), Style::default().fg(MUTED)),
                ]));
            }
            match entry.choice {
                HomeChoice::Session(choice) => {
                    let [mut first, mut second] =
                        super::super::layout::lines(&choice.row, now, width, focused);
                    first.spans.insert(0, caret(focused));
                    second.spans.insert(0, Span::raw("    "));
                    lines.extend([first, second]);
                }
                HomeChoice::Agent(row) => lines.extend(agent_lines(
                    row,
                    width,
                    focused,
                    state.selected_agents.contains(&row.slug),
                )),
            }
            ListItem::new(lines)
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("  No matching sessions or agents").style(Style::default().fg(MUTED)),
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
    let confirmation = state.confirmation_text();
    let deletion = delete_notice(state);
    let footer = confirmation
        .as_deref()
        .map(|prompt| (prompt, WARNING))
        .or_else(|| deletion.as_deref().map(|prompt| (prompt, ERROR)))
        .or_else(|| state.notice.as_ref().map(|notice| (notice.as_str(), ERROR)))
        .unwrap_or_else(|| (help(state), MUTED));
    frame.render_widget(
        Paragraph::new(format!("{} · {position}", footer.0)).style(Style::default().fg(footer.1)),
        help_area,
    );
}

fn agent_lines(
    row: &crate::cli::agents::AgentRow,
    width: usize,
    focused: bool,
    selected: bool,
) -> [Line<'static>; 2] {
    let source = match row.kind {
        AgentKind::Configured => "configured",
        AgentKind::NativeProfile => "native profile",
        AgentKind::Generic => "generic",
    };
    let label = format!(
        "{} · {source}",
        crate::cli::agents::harness_name(row.harness)
    );
    let name_width = row.slug.chars().count();
    let padding = width
        .saturating_sub(4 + name_width + label.chars().count())
        .max(2);
    let name_style = if focused {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    [
        Line::from(vec![
            caret(focused),
            Span::styled(
                if selected { "✓ " } else { "＋ " },
                Style::default().fg(crate::console_style::harness_ratatui_color(row.harness)),
            ),
            Span::styled(row.slug.clone(), name_style),
            Span::raw(" ".repeat(padding)),
            Span::styled(label, Style::default().fg(MUTED)),
        ]),
        Line::from(Span::styled(
            format!("    {}", row.summary(width.saturating_sub(4))),
            Style::default().fg(MUTED),
        )),
    ]
}

fn caret(focused: bool) -> Span<'static> {
    Span::styled(
        if focused { "❯ " } else { "  " },
        Style::default().fg(if focused { ACCENT } else { MUTED }),
    )
}

fn delete_notice(state: &PickerState) -> Option<String> {
    match state.pending_delete.as_ref()? {
        PendingDelete::Nothing { slug } => Some(format!(
            "{} is a generic agent — nothing to delete · any key cancels",
            slug
        )),
        PendingDelete::ChooseScope { index } => Some(format!(
            "Delete {}: a) agent config · p) native profile · b) both · esc cancel",
            state.agent(*index).slug
        )),
        PendingDelete::Confirm { plan } => {
            let what = if let [(index, scope)] = plan.as_slice() {
                let target = match scope {
                    crate::cli::interactive::agent_picker::DeleteScope::Agent => "agent config",
                    crate::cli::interactive::agent_picker::DeleteScope::Profile => "native profile",
                    crate::cli::interactive::agent_picker::DeleteScope::Both => "agent and profile",
                };
                format!("{target} for {}", state.agent(*index).slug)
            } else {
                format!("{} selected agents", plan.len())
            };
            Some(format!("Delete {what}? y/d confirm · esc cancel"))
        }
    }
}

fn help(state: &PickerState) -> &'static str {
    if state.filtering {
        return "enter open · type search · ↑↓ move · esc clear";
    }
    let Some(index) = state.current_choice() else {
        return "/ search · -/+ history · p project · ↑↓ · esc";
    };
    match state.choices[index] {
        HomeChoice::Session(_) => {
            "enter attach/restart · ⇧K kill · -/+ history · p project · / search · ↑↓ · esc"
        }
        HomeChoice::Agent(_) => {
            "enter launch · e edit · d delete · space select · p project · / search · ↑↓ · esc"
        }
    }
}

fn draw_projects(frame: &mut Frame<'_>, projects: &super::project::ProjectPicker) {
    let area = frame.area();
    let [title_area, options_area, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let query = if projects.query.is_empty() {
        Span::styled("type to filter", Style::default().fg(MUTED))
    } else {
        Span::styled(projects.query.as_str(), Style::default().fg(ACCENT))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Projects", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  Filter: "),
            query,
        ])),
        title_area,
    );
    let items = projects
        .window(usize::from(options_area.height))
        .map(|(position, option)| {
            let focused = position == projects.cursor;
            let mut spans = vec![
                caret(focused),
                Span::styled(
                    option.name.clone(),
                    if focused {
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ];
            if let Some(path) = option.path.as_deref() {
                spans.push(Span::styled(
                    format!("  {path}"),
                    Style::default().fg(MUTED),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("  No matching projects").style(Style::default().fg(MUTED)),
            options_area,
        );
    } else {
        frame.render_widget(List::new(items), options_area);
    }
    frame.render_widget(
        Paragraph::new("enter select · type filter · ↑↓ move · esc back")
            .style(Style::default().fg(MUTED)),
        help_area,
    );
}
