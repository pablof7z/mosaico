use super::{delete::PendingDelete, state::PickerTab, PickerState};
use crate::cli::interactive::session_picker::HomeChoice;
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

mod agent;

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
    let query = if state.query.is_empty() {
        Span::styled("type to search", Style::default().fg(MUTED))
    } else {
        Span::styled(state.query.as_str(), Style::default().fg(ACCENT))
    };
    let mut title = vec![
        Span::styled("Mosaico", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        tab("Sessions", sessions, state.tab == PickerTab::Sessions),
        Span::raw(" "),
        tab("Start a session", agents, state.tab == PickerTab::Agents),
    ];
    title.extend([
        Span::styled(
            "  Search: ",
            if !state.query.is_empty() {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        ),
        query,
    ]);
    if state.tab == PickerTab::Sessions {
        title.extend([
            Span::raw("  History: "),
            Span::styled(state.range.label(), Style::default().fg(ACCENT)),
            Span::raw("  Project: "),
            Span::styled(state.project_label(), Style::default().fg(ACCENT)),
        ]);
    }
    frame.render_widget(Paragraph::new(Line::from(title)), title_area);

    let width = usize::from(options_area.width.saturating_sub(4));
    let now = crate::util::now_secs();
    let items = state
        .window(usize::from(options_area.height))
        .into_iter()
        .map(|entry| {
            let focused = entry.position == state.cursor;
            let mut lines = Vec::with_capacity(2);
            match entry.choice {
                HomeChoice::Session(choice) => {
                    let [mut first, mut second] =
                        super::super::layout::lines(&choice.row, now, width, focused);
                    first.spans.insert(0, caret(focused));
                    second.spans.insert(0, Span::raw("    "));
                    lines.extend([first, second]);
                }
                HomeChoice::Agent(row) => lines.extend(agent::lines(
                    row,
                    width,
                    focused,
                    state.selected_agents.contains(&row.slug),
                )),
            }
            ListItem::new(lines).style(if focused {
                Style::default().bg(SELECTED_BG)
            } else {
                Style::default()
            })
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        let empty = match state.tab {
            PickerTab::Sessions => "  No matching sessions",
            PickerTab::Agents => "  No matching launchable agents",
        };
        frame.render_widget(
            Paragraph::new(empty).style(Style::default().fg(MUTED)),
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
    let kind = (confirmation.is_none() && deletion.is_none() && state.notice.is_none())
        .then(|| {
            state
                .current_agent_index()
                .map(|index| agent::kind_label(state.agent(index).kind))
        })
        .flatten();
    let mut footer_spans = Vec::new();
    if let Some(kind) = kind {
        footer_spans.push(Span::styled(kind, Style::default().fg(ACCENT)));
        footer_spans.push(Span::styled(" · ", Style::default().fg(MUTED)));
    }
    footer_spans.push(Span::styled(
        format!("{} · {position}", footer.0),
        Style::default().fg(footer.1),
    ));
    frame.render_widget(Paragraph::new(Line::from(footer_spans)), help_area);
}

fn tab(label: &str, count: usize, active: bool) -> Span<'static> {
    Span::styled(
        format!(" {label} {count} "),
        if active {
            Style::default()
                .fg(Color::White)
                .bg(SELECTED_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MUTED)
        },
    )
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
    if !state.query.is_empty() {
        return "enter open · type search · ↑↓ move · esc clear";
    }
    let Some(index) = state.current_choice() else {
        return "type search · tab switch · ↑↓ · esc";
    };
    match state.choices[index] {
        HomeChoice::Session(_) => {
            "enter attach/restart · ctrl-k kill · ctrl-+/- history · ctrl-p project · tab start · type search · ↑↓ · esc"
        }
        HomeChoice::Agent(_) => {
            "enter launch · ctrl-e edit · ctrl-d delete · ctrl-space select · tab sessions · type search · ↑↓ · esc"
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
