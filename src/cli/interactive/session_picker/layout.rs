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
    let status = state_status(row.state, row.state_since, now);
    let fixed_width = 2 + handle.width() + 3 + status.width();
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
    first.push(Span::styled(
        status,
        Style::default().fg(state_color(row.state)),
    ));

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
                    crate::channel_ref::format_channel_ref(
                        &workspace.id,
                        &[channel.name.clone()],
                    ),
                    Style::default().fg(CHANNEL),
                ));
            }
        }
    }
    spans
}

fn state_status(state: crate::session_state::SessionState, since: u64, now: u64) -> String {
    if state == crate::session_state::SessionState::Working || since == 0 {
        state.to_string()
    } else {
        format!("{state} · {}", crate::util::relative_time(since, now))
    }
}

impl SessionRow {
    fn work(&self) -> String {
        let title = self.title.trim();
        let activity = self.activity.trim();
        let work = if title.is_empty() {
            String::new()
        } else if activity.is_empty() || activity == title {
            title.to_string()
        } else {
            format!("{title} — {activity}")
        };
        let Some(outcome) = self
            .native_outcome
            .as_ref()
            .filter(|outcome| outcome.is_failure())
        else {
            return work;
        };
        let detail = outcome.error_message.trim();
        let failure = if detail.is_empty() {
            format!("native {}", outcome.outcome.replace('_', " "))
        } else {
            format!("native {}: {detail}", outcome.outcome.replace('_', " "))
        };
        if work.is_empty() {
            failure
        } else {
            format!("{work} — {failure}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::interactive::session_picker::data::{ChannelRef, WorkspaceGroup};

    #[test]
    fn row_is_always_two_lines_with_canonical_state_and_without_transport() {
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
            state_since: 90,
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
        assert!(first.contains("mosaico: #mosaico-root/ideas"));
        assert!(first.ends_with("working"));
        assert!(!first.contains("PTY"));
        assert_eq!(second, "Implement session picker — running tests");
    }

    #[test]
    fn idle_suspended_and_offline_states_include_semantic_age() {
        for (state, since, now, expected) in [
            (
                crate::session_state::SessionState::Idle,
                40,
                100,
                "idle · 1 min ago",
            ),
            (
                crate::session_state::SessionState::Suspended,
                3_700,
                7_300,
                "suspended · 1 hour ago",
            ),
            (
                crate::session_state::SessionState::Offline,
                7_300,
                14_500,
                "offline · 2 hours ago",
            ),
        ] {
            let row = SessionRow {
                handle: "delta-codex".into(),
                state,
                state_since: since,
                ..SessionRow::default()
            };
            let rendered = lines(&row, now, 100, false);
            let first = rendered[0]
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            assert!(first.ends_with(expected), "{first}");
        }
    }

    #[test]
    fn native_failure_is_shown_without_changing_idle_state() {
        let row = SessionRow {
            handle: "delta-codex".into(),
            title: "Review change".into(),
            state: crate::session_state::SessionState::Idle,
            state_since: 40,
            native_outcome: Some(
                crate::cli::interactive::session_picker::data::NativeOutcomeRow {
                    outcome: "failed".into(),
                    error_message: "unsupported model".into(),
                },
            ),
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
        assert!(first.ends_with("idle · 1 min ago"), "{first}");
        assert_eq!(second, "Review change — native failed: unsupported model");
    }
}
