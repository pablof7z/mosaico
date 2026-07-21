use super::data::{SessionRow, WorkspaceGroup};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

const ACCENT: Color = Color::Indexed(45);
const WORKING: Color = Color::Indexed(78);
const IDLE: Color = Color::Indexed(45);
const SUSPENDED: Color = Color::Indexed(214);
const OFFLINE: Color = Color::Indexed(245);
const CHANNEL: Color = Color::Indexed(75);
const MUTED: Color = Color::Indexed(245);

pub(super) fn lines(row: &SessionRow, now: u64, width: usize, focused: bool) -> [Line<'static>; 2] {
    let greyed = row.transport == "acp";
    let handle = format!("@{}", row.handle);
    let scope = scope_spans(&row.workspaces);
    let scope_width = scope.iter().map(|span| span.content.width()).sum::<usize>();
    let age = seen(row.last_seen, now);
    let fixed_width = 2 + handle.width() + 3 + age.width();
    let padding = width.saturating_sub(fixed_width + scope_width).max(1);

    let mut first = vec![
        Span::styled("● ", Style::default().fg(state_color(row.state))),
        Span::styled(
            handle,
            if focused {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else if greyed {
                Style::default().fg(MUTED).add_modifier(Modifier::DIM)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            },
        ),
        Span::raw("   "),
    ];
    first.extend(scope);
    first.push(Span::raw(" ".repeat(padding)));
    first.push(Span::styled(age, Style::default().fg(MUTED)));

    let work_style = if greyed {
        Style::default().fg(MUTED).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(MUTED)
    };
    [
        Line::from(first),
        Line::from(Span::styled(row.work(), work_style)),
    ]
}

fn state_color(state: crate::session_state::SessionState) -> Color {
    match state {
        crate::session_state::SessionState::Working => WORKING,
        crate::session_state::SessionState::Idle => IDLE,
        crate::session_state::SessionState::Suspended => SUSPENDED,
        crate::session_state::SessionState::Offline => OFFLINE,
    }
}

fn scope_spans(workspaces: &[WorkspaceGroup]) -> Vec<Span<'static>> {
    if workspaces.is_empty() {
        return vec![Span::styled("(no workspace)", Style::default().fg(MUTED))];
    }
    let mut spans = Vec::new();
    for (index, workspace) in workspaces.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(MUTED)));
        }
        spans.push(Span::styled(
            workspace.name.clone(),
            Style::default()
                .fg(crate::console_style::workspace_ratatui_color(&workspace.id))
                .add_modifier(Modifier::BOLD),
        ));
        let channels = workspace
            .channels
            .iter()
            .filter(|channel| channel.id != workspace.id)
            .collect::<Vec<_>>();
        if !channels.is_empty() {
            spans.push(Span::raw(": "));
            for (channel_index, channel) in channels.iter().enumerate() {
                if channel_index > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(
                    format!("#{}", channel.name),
                    Style::default().fg(CHANNEL),
                ));
            }
        }
    }
    spans
}

fn seen(last_seen: u64, now: u64) -> String {
    if last_seen == 0 {
        "unknown".to_string()
    } else if now.saturating_sub(last_seen) < 60 {
        "online".to_string()
    } else {
        crate::util::relative_time(last_seen, now)
    }
}

impl SessionRow {
    fn work(&self) -> String {
        let title = self.title.trim();
        if title.is_empty() {
            return String::new();
        }
        let activity = self.activity.trim();
        if activity.is_empty() || activity == title {
            title.to_string()
        } else {
            format!("{title} — {activity}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::interactive::session_picker::data::{ChannelRef, WorkspaceGroup};

    #[test]
    fn row_is_always_two_lines_without_textual_state_or_transport() {
        let row = SessionRow {
            handle: "delta-codex".into(),
            workspaces: vec![WorkspaceGroup {
                id: "mosaico-root".into(),
                name: "mosaico".into(),
                channels: vec![ChannelRef {
                    id: "ideas".into(),
                    name: "ideas".into(),
                }],
                ..WorkspaceGroup::default()
            }],
            title: "Implement session picker".into(),
            activity: "running tests".into(),
            state: crate::session_state::SessionState::Working,
            last_seen: 98,
            ..SessionRow::default()
        };

        let rendered = lines(&row, 100, 100, true);
        let first = rendered[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        let second = rendered[1]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(first.contains("● @delta-codex"));
        assert!(first.contains("mosaico: #ideas"));
        assert!(!first.contains("working"));
        assert!(!first.contains("PTY"));
        assert_eq!(second, "Implement session picker — running tests");
    }

    #[test]
    fn recent_sessions_are_online_and_empty_titles_stay_empty() {
        let row = SessionRow {
            handle: "delta-codex".into(),
            activity: "running tests".into(),
            last_seen: 98,
            ..SessionRow::default()
        };

        let rendered = lines(&row, 100, 100, false);
        let first = rendered[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let second = rendered[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(first.ends_with("online"), "{first}");
        assert_eq!(second, "");
    }
}
