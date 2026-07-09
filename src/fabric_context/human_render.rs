use crate::fabric_context::model::*;
use owo_colors::OwoColorize as _;
use std::fmt::Write as _;

pub(in crate::fabric_context) fn render_human_view(view: &FabricView, color: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", style(&view.workspace.name, color, Style::Title));
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
        return out;
    }

    render_agents(&mut out, &view.agents, color);
    for channel in &view.channels {
        render_channel(&mut out, channel, color);
    }
    render_unjoined(&mut out, &view.unjoined, color);
    render_important(&mut out, &view.important, color);
    render_warnings(&mut out, &view.warnings, color);
    out
}

fn render_agents(out: &mut String, agents: &[AgentRow], color: bool) {
    if agents.is_empty() {
        return;
    }
    let _ = writeln!(out, "{}", style("Available agents", color, Style::Header));
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

fn render_channel(out: &mut String, channel: &ChannelBlock, color: bool) {
    let name = format!("#{}", channel.name);
    if channel.about.is_empty() {
        let _ = writeln!(out, "{}", style(&name, color, Style::Channel));
    } else {
        let _ = writeln!(
            out,
            "{}  {}",
            style(&name, color, Style::Channel),
            channel.about
        );
    }
    render_members(out, &channel.members, color);
    render_presence(out, &channel.presence, color);
    render_subchannels(out, &channel.subchannels, color);
    render_messages(out, channel, color);
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
        let role = dim(&format!("{:<6}", m.role), color);
        let _ = writeln!(
            out,
            "    {} {} {:<12} {} {}",
            style(&reference, color, Style::Agent),
            role,
            status_text(&m.status, color),
            dim("seen", color),
            dim(&m.seen, color)
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
            "    {}  {:<12} {} {}",
            style(&reference, color, Style::Agent),
            status_text(&p.status, color),
            dim("seen", color),
            dim(&p.seen, color)
        );
    }
}

fn render_subchannels(out: &mut String, subs: &[ChannelSummaryRow], color: bool) {
    if subs.is_empty() {
        return;
    }
    let _ = writeln!(out, "  {}", dim("Subchannels", color));
    for ch in subs {
        let name = format!("#{}", ch.name);
        if ch.about.is_empty() {
            let _ = writeln!(out, "    {}", style(&name, color, Style::Channel));
        } else {
            let _ = writeln!(
                out,
                "    {}  {}",
                style(&name, color, Style::Channel),
                ch.about
            );
        }
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
                        "truncated; run `tenex-edge channel read --id {}`",
                        crate::util::short_id(&m.id)
                    ),
                    color
                )
            );
        }
    }
}

fn render_unjoined(out: &mut String, unjoined: &[UnjoinedChannelRow], color: bool) {
    if unjoined.is_empty() {
        return;
    }
    let _ = writeln!(out, "{}", style("Other channels", color, Style::Header));
    for ch in unjoined {
        let name = format!("#{}", ch.name);
        if ch.about.is_empty() {
            let _ = writeln!(
                out,
                "  {}  {} {}",
                style(&name, color, Style::Channel),
                dim("last active", color),
                dim(&ch.last_active, color)
            );
        } else {
            let _ = writeln!(
                out,
                "  {}  {} {} - {}",
                style(&name, color, Style::Channel),
                dim("last active", color),
                dim(&ch.last_active, color),
                ch.about
            );
        }
    }
    out.push('\n');
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
            style(&format!("#{}", row.channel), color, Style::Channel)
        );
    }
    out.push('\n');
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

fn status_text(status: &str, color: bool) -> String {
    let trimmed = status.trim();
    let label = if trimmed.is_empty() {
        "unknown"
    } else {
        trimmed
    };
    match label {
        "working" => style(label, color, Style::Good),
        "idle" => style(label, color, Style::Idle),
        "offline" | "unknown" => dim(label, color),
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
