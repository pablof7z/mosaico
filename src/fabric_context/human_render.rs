use crate::fabric_context::model::*;
use owo_colors::OwoColorize as _;
use std::fmt::Write as _;

mod all_workspaces;
mod channel;

pub(in crate::fabric_context) use all_workspaces::render_human_views;
use channel::render_channel;

pub(in crate::fabric_context) fn render_human_view(view: &FabricView, color: bool) -> String {
    let mut out = String::new();
    render_human_workspace(&mut out, view, &view.agents, "Available agents", color);
    out
}

pub(super) fn render_human_workspace(
    out: &mut String,
    view: &FabricView,
    agents: &[AgentRow],
    agents_label: &str,
    color: bool,
) {
    let workspace =
        crate::console_style::paint_workspace(&view.workspace.name, &view.workspace.channel, color);
    let _ = writeln!(out, "{}", style(&workspace, color, Style::Title));
    if !view.workspace.about.is_empty() {
        let _ = writeln!(out, "{}", dim(&view.workspace.about, color));
    }
    out.push('\n');
    if view.is_quiet_delta() {
        let _ = writeln!(
            out,
            "{}",
            dim(
                "Nothing new since your last check. The fabric surfaces only what \
                 changed — your channels and members are unchanged, not gone.",
                color
            )
        );
        return;
    }

    render_agents(out, agents, agents_label, color);
    render_workspace_tree(out, view.root.as_ref(), &view.channels, color);
    for activity in &view.other_workspaces {
        out.push('\n');
        let workspace = crate::console_style::paint_workspace(
            &activity.workspace.name,
            &activity.workspace.channel,
            color,
        );
        let _ = writeln!(out, "{}", style(&workspace, color, Style::Title));
        if !activity.workspace.about.is_empty() {
            let _ = writeln!(out, "{}", dim(&activity.workspace.about, color));
        }
        out.push('\n');
        render_workspace_tree(out, activity.root.as_ref(), &activity.channels, color);
    }
    render_important(out, &view.important, color);
    render_warnings(out, &view.warnings, color);
}

fn render_workspace_tree(
    out: &mut String,
    root: Option<&ChannelBlock>,
    channels: &[ChannelBlock],
    color: bool,
) {
    if let Some(root) = root {
        render_channel_body(out, root, color);
    }
    if root.is_some_and(|root| !root.children.is_empty()) || !channels.is_empty() {
        let _ = writeln!(out, "{}", style("Channels", color, Style::Header));
        if let Some(root) = root {
            for channel in &root.children {
                render_channel(out, channel, color, 2);
            }
        }
    }
    for channel in channels {
        render_channel(out, channel, color, 2);
    }
}

pub(super) fn render_agents(out: &mut String, agents: &[AgentRow], label: &str, color: bool) {
    if agents.is_empty() {
        return;
    }
    let _ = writeln!(out, "{}", style(label, color, Style::Header));
    for a in agents {
        let name = format!("@{}", a.reference);
        if a.about.is_empty() {
            let _ = writeln!(out, "  {}", style(&name, color, Style::Agent));
        } else {
            let _ = writeln!(out, "  {}  {}", style(&name, color, Style::Agent), a.about);
        }
    }
    out.push('\n');
}

fn render_members(out: &mut String, members: &[MemberRow], color: bool) {
    if members.is_empty() {
        return;
    }
    let width = members
        .iter()
        .map(|m| m.reference.len() + 1)
        .max()
        .unwrap_or(0)
        .max(8);
    let _ = writeln!(out, "  {}", dim("Members", color));
    for m in members {
        let reference = pad_ref(&m.reference, width);
        let _ = writeln!(
            out,
            "    {}  {:<12} {} {} {}",
            style(&reference, color, Style::Agent),
            state_text(m.state, color),
            m.status,
            dim("since", color),
            dim(&m.since, color)
        );
    }
}

fn render_presence(out: &mut String, presence: &[PresenceRow], color: bool) {
    if presence.is_empty() {
        return;
    }
    let width = presence
        .iter()
        .map(|p| p.reference.len() + 1)
        .max()
        .unwrap_or(0)
        .max(8);
    let _ = writeln!(out, "  {}", dim("Recent presence", color));
    for p in presence {
        let reference = pad_ref(&p.reference, width);
        let _ = writeln!(
            out,
            "    {}  {:<12} {} {} {}",
            style(&reference, color, Style::Agent),
            state_text(p.state, color),
            p.status,
            dim("since", color),
            dim(&p.since, color)
        );
    }
}

fn render_messages(out: &mut String, channel: &ChannelBlock, color: bool) {
    if channel.messages.is_empty() && channel.omitted == 0 {
        return;
    }
    let _ = writeln!(out, "  {}", dim("Messages", color));
    if channel.omitted > 0 {
        let _ = writeln!(
            out,
            "    {}",
            dim(
                &format!(
                    "{} older message(s) omitted from the last 4h",
                    channel.omitted
                ),
                color
            )
        );
    }
    for m in &channel.messages {
        let from = format!("@{}", m.from);
        let marker = if m.mention {
            format!("{} ", style("mention", color, Style::Warning))
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "    {} {}{}",
            style(&from, color, Style::Agent),
            marker,
            m.body
        );
        if m.truncated {
            let _ = writeln!(
                out,
                "      {}",
                dim(
                    &format!(
                        "truncated; run `mosaico channel read --id {}`",
                        crate::util::short_id(&m.id)
                    ),
                    color
                )
            );
        }
    }
}

fn render_important(out: &mut String, important: &[ImportantRow], color: bool) {
    if important.is_empty() {
        return;
    }
    let _ = writeln!(out, "{}", style("Important", color, Style::Header));
    for row in important {
        let _ = writeln!(
            out,
            "  {} in {}",
            style(
                &crate::util::short_id(&row.message_id),
                color,
                Style::Warning
            ),
            style(&format!("#{}", row.channel_ref), color, Style::Channel)
        );
    }
    out.push('\n');
}

pub(super) fn render_channel_body(out: &mut String, channel: &ChannelBlock, color: bool) {
    render_members(out, &channel.members, color);
    render_presence(out, &channel.presence, color);
    render_messages(out, channel, color);
}

fn render_warnings(out: &mut String, warnings: &[WarningRow], color: bool) {
    if warnings.is_empty() {
        return;
    }
    let _ = writeln!(out, "{}", style("Warnings", color, Style::Warning));
    for row in warnings {
        let _ = writeln!(out, "  {}", row.text);
    }
    out.push('\n');
}

fn pad_ref(reference: &str, width: usize) -> String {
    format!("{:<width$}", format!("@{reference}"), width = width)
}

fn state_text(state: crate::session_state::SessionState, color: bool) -> String {
    let label = state.as_str();
    match label {
        "working" => style(label, color, Style::Good),
        "idle" | "suspended" => style(label, color, Style::Idle),
        "offline" => dim(label, color),
        _ => style(label, color, Style::Good),
    }
}

#[derive(Clone, Copy)]
enum Style {
    Agent,
    Channel,
    Good,
    Header,
    Idle,
    Title,
    Warning,
}

fn style(text: &str, color: bool, style: Style) -> String {
    if !color {
        return text.to_string();
    }
    match style {
        Style::Agent => text.cyan().to_string(),
        Style::Channel => text.blue().bold().to_string(),
        Style::Good => text.green().to_string(),
        Style::Header => text.bold().to_string(),
        Style::Idle => text.yellow().to_string(),
        Style::Title => text.bold().underline().to_string(),
        Style::Warning => text.red().bold().to_string(),
    }
}

fn dim(text: &str, color: bool) -> String {
    if color {
        text.dimmed().to_string()
    } else {
        text.to_string()
    }
}
