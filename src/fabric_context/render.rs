//! Sole agent-facing XML serializer; node selection happens before rendering,
//! so this module cannot vary by cursor, caller, or delivery surface.

use crate::fabric_context::model::*;
use crate::fabric_context::xml::{attr, text};
use std::fmt::Write as _;
pub(in crate::fabric_context) fn render_view(view: &FabricView) -> String {
    let mut out = String::from("<mosaico>");
    render_self(&mut out, view.self_row.as_ref());
    render_hosts(&mut out, view.hosts.as_deref());
    render_workspaces(&mut out, view.workspaces.as_deref());
    render_important(&mut out, &view.important);
    render_reactions(&mut out, &view.reactions, view.reactions_omitted);
    render_warnings(&mut out, &view.warnings);
    render_notices(&mut out, &view.notices);
    out.push_str("\n</mosaico>");
    out
}
fn render_self(out: &mut String, row: Option<&SelfRow>) {
    let Some(row) = row else {
        return;
    };
    let name = attr(row.name.trim_start_matches('@'));
    let host = attr(&row.host);
    let headless = if row.headless { "on" } else { "off" };
    let _ = write!(
        out,
        "\n  <self name=\"@{name}\" host=\"{host}\" headless=\"{headless}\""
    );
    if !row.title.is_empty() {
        let _ = write!(out, " title=\"{}\"", attr(&row.title));
    }
    out.push_str(" />");
    if !row.hint.is_empty() {
        let _ = write!(out, "\n  <notice>{}</notice>", text(&row.hint));
    }
}
fn render_hosts(out: &mut String, hosts: Option<&[HostRow]>) {
    let Some(hosts) = hosts else {
        return;
    };
    out.push_str("\n  <hosts>");
    for host in hosts {
        let _ = write!(out, "\n    <host name=\"{}\">", attr(&host.name));
        out.push_str("\n      <agents>");
        for agent in &host.agents {
            let _ = write!(out, "\n        <agent ref=\"{}\"", attr(&agent.reference));
            if !agent.about.is_empty() {
                let _ = write!(out, " about=\"{}\"", attr(&agent.about));
            }
            out.push_str(" />");
        }
        out.push_str("\n      </agents>\n    </host>");
    }
    out.push_str("\n  </hosts>");
}
fn render_workspaces(out: &mut String, workspaces: Option<&[WorkspaceView]>) {
    let Some(workspaces) = workspaces else {
        return;
    };
    out.push_str("\n  <workspaces>");
    for workspace in workspaces {
        render_workspace(out, workspace);
    }
    out.push_str("\n  </workspaces>");
}
fn render_workspace(out: &mut String, workspace: &WorkspaceView) {
    let _ = write!(out, "\n    <workspace name=\"{}\"", attr(&workspace.name));
    if !workspace.about.is_empty() {
        let _ = write!(out, " about=\"{}\"", attr(&workspace.about));
    }
    let _ = write!(out, " hosts=\"{}\">", attr(&workspace.hosts.join(",")));
    if let Some(root) = &workspace.root {
        render_channel(out, root, 6);
    }
    for channel in &workspace.channels {
        render_channel(out, channel, 6);
    }
    out.push_str("\n    </workspace>");
}
fn render_channel(out: &mut String, channel: &ChannelBlock, indent: usize) {
    let pad = " ".repeat(indent);
    let name = attr(&channel.name);
    let id = attr(&channel.id);
    let _ = write!(out, "\n{pad}<channel name=\"{name}\" id=\"{id}\"");
    if !channel.about.is_empty() {
        let _ = write!(out, " about=\"{}\"", attr(&channel.about));
    }
    if let Some(count) = channel.member_count {
        let _ = write!(out, " members=\"{count}\"");
    }
    if let Some(last_active) = &channel.last_active {
        let _ = write!(out, " last-active=\"{}\"", attr(last_active));
    }
    if channel.is_compact() {
        out.push_str(" />");
        return;
    }
    out.push('>');
    render_members(out, &channel.members, indent + 2);
    render_presence(out, &channel.presence, indent + 2);
    render_messages(out, channel, indent + 2);
    for child in &channel.children {
        render_channel(out, child, indent + 2);
    }
    let _ = write!(out, "\n{pad}</channel>");
}
fn render_members(out: &mut String, members: &[MemberRow], indent: usize) {
    if members.is_empty() {
        return;
    }
    let pad = " ".repeat(indent);
    let child_pad = " ".repeat(indent + 2);
    let _ = write!(out, "\n{pad}<members>");
    for member in members {
        let tag = match member.kind {
            MemberKind::Agent => "agent",
            MemberKind::Human => "human",
        };
        let name = attr(member.name.trim_start_matches('@'));
        let _ = write!(out, "\n{child_pad}<{tag} name=\"@{name}\"");
        if let Some(state) = member.state {
            let _ = write!(out, " state=\"{}\"", state.as_str());
        }
        if !member.status.is_empty() {
            let _ = write!(out, " status=\"{}\"", attr(&member.status));
        }
        let _ = write!(out, " since=\"{}\" />", attr(&member.since));
    }
    let _ = write!(out, "\n{pad}</members>");
}
fn render_presence(out: &mut String, presence: &[PresenceRow], indent: usize) {
    if presence.is_empty() {
        return;
    }
    let pad = " ".repeat(indent);
    let child_pad = " ".repeat(indent + 2);
    let _ = write!(out, "\n{pad}<recent-presence>");
    for status in presence {
        if !status.status.is_empty() {
            let name = attr(status.name.trim_start_matches('@'));
            let state = status.state.as_str();
            let status_text = attr(&status.status);
            let since = attr(&status.since);
            let _ = write!(
                out,
                "\n{child_pad}<status name=\"@{name}\" state=\"{state}\" \
                 text=\"{status_text}\" since=\"{since}\" />"
            );
        }
        if let Some(failure) = &status.native_failure {
            let name = attr(status.name.trim_start_matches('@'));
            let outcome = attr(&failure.outcome);
            let message = attr(&failure.message);
            let since = attr(&failure.since);
            let _ = write!(
                out,
                "\n{child_pad}<native-outcome name=\"@{name}\" outcome=\"{outcome}\" \
                 text=\"{message}\" since=\"{since}\" />"
            );
        }
    }
    let _ = write!(out, "\n{pad}</recent-presence>");
}
fn render_messages(out: &mut String, channel: &ChannelBlock, indent: usize) {
    if channel.messages.is_empty() && channel.omitted == 0 {
        return;
    }
    let pad = " ".repeat(indent);
    let child_pad = " ".repeat(indent + 2);
    let detail_pad = " ".repeat(indent + 4);
    let _ = write!(out, "\n{pad}<chatter>");
    if channel.omitted > 0 {
        let _ = write!(
            out,
            "\n{child_pad}<omitted count=\"{}\" window=\"last 4h\" />",
            channel.omitted
        );
    }
    for message in &channel.messages {
        if message.mention {
            render_mention_message(out, message, &child_pad);
            continue;
        }
        let short = crate::util::short_id(&message.id);
        let from = attr(&message.from);
        let id = attr(&short);
        let _ = write!(out, "\n{child_pad}<message from=\"@{from}\" id=\"{id}\"");
        if !message.recipients.is_empty() {
            let recipients = message
                .recipients
                .iter()
                .map(|recipient| format!("@{recipient}"))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = write!(out, " for=\"{}\"", attr(&recipients));
        }
        let _ = write!(out, " age=\"{}\">", attr(&message.age));
        out.push_str(&text(&message.body));
        if message.truncated {
            let _ = write!(
                out,
                "\n{detail_pad}[message truncated; run `mosaico channel read --id {}`]",
                text(&short)
            );
        }
        out.push_str("</message>");
    }
    let _ = write!(out, "\n{pad}</chatter>");
}
fn render_mention_message(out: &mut String, message: &MessageRow, pad: &str) {
    let short = crate::util::short_id(&message.id);
    let from = attr(&message.from);
    let id = attr(&short);
    let body = text(&message.body);
    let _ = write!(
        out,
        "\n{pad}<message from=\"@{from}\" id=\"{id}\">{body}</message>"
    );
    if message.needs_reply_nudge {
        let _ = write!(
            out,
            "\n{pad}Need a follow-up? Read `skills/mosaico/references/coordination-guide.md`."
        );
    }
}
fn render_important(out: &mut String, rows: &[ImportantRow]) {
    if rows.is_empty() {
        return;
    }
    out.push_str("\n  <important>");
    for row in rows {
        let channel = attr(&row.channel_ref);
        let message_id = attr(&crate::util::short_id(&row.message_id));
        let _ = write!(
            out,
            "\n    <mention channel=\"{channel}\" message_id=\"{message_id}\" />"
        );
    }
    out.push_str("\n  </important>");
}
fn render_reactions(out: &mut String, rows: &[ReactionRow], omitted: usize) {
    if rows.is_empty() && omitted == 0 {
        return;
    }
    out.push_str("\n  <reactions>");
    for row in rows {
        let reactors = row
            .reactors
            .iter()
            .map(|reactor| format!("@{}", text(reactor)))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(
            out,
            "\n    {} {} on your message \"{}\" ({})",
            reactors,
            text(&row.emoji),
            text(&row.target_snippet),
            text(&row.age)
        );
    }
    if omitted > 0 {
        let _ = write!(out, "\n    <omitted count=\"{omitted}\" />");
    }
    out.push_str("\n  </reactions>");
}
fn render_warnings(out: &mut String, rows: &[WarningRow]) {
    if rows.is_empty() {
        return;
    }
    out.push_str("\n  <warnings>");
    for row in rows {
        let _ = write!(out, "\n    <warning>{}</warning>", text(&row.text));
    }
    out.push_str("\n  </warnings>");
}
fn render_notices(out: &mut String, rows: &[NoticeRow]) {
    for NoticeRow::NoNewActivity { workspace } in rows {
        let _ = write!(
            out,
            "\n  <no-new-activity workspace=\"{}\">\
             \n    Nothing new since your last check. The fabric surfaces only what \
             changed — your channels, members, and messages are unchanged, not gone.\
             \n  </no-new-activity>",
            attr(workspace)
        );
    }
}
