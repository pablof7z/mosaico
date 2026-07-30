//! Sole agent-facing XML serializer; node selection happens before rendering,
//! so this module cannot vary by cursor, caller, or delivery surface.

mod activity;
use activity::{render_important, render_messages, render_reactions};

use crate::agent_xml::{attr, text};
use crate::fabric_context::model::*;
use std::fmt::Write as _;
pub(in crate::fabric_context) fn render_view(view: &FabricView) -> String {
    let mut out = String::from("<mosaico>");
    render_self(&mut out, view.self_row.as_ref());
    render_hosts(&mut out, view.hosts.as_deref());
    render_channels(&mut out, view.workspaces.as_deref(), view.now);
    render_important(&mut out, &view.important);
    render_reactions(&mut out, &view.reactions, view.reactions_omitted);
    render_warnings(&mut out, &view.warnings);
    render_notices(&mut out, &view.notices);
    if view.coordination_guide_reminder {
        let _ = write!(
            out,
            "\n  <notice>{}</notice>",
            text(crate::reconcile::COORDINATION_GUIDE_REMINDER)
        );
    }
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
    if !row.workspace.is_empty() {
        let _ = write!(out, " workspace=\"{}\"", attr(&row.workspace));
    }
    if !row.branch.is_empty() {
        let _ = write!(out, " branch=\"{}\"", attr(&row.branch));
    }
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
    if hosts.is_empty() {
        return;
    }
    out.push_str("\n  <hosts>");
    for host in hosts {
        let _ = write!(out, "\n    <host name=\"{}\">", attr(&host.name));
        if !host.roots.is_empty() {
            out.push_str("\n      Workspaces:");
            for root in &host.roots {
                let _ = write!(out, "\n      * {}", text(root));
            }
        }
        if !host.agents.is_empty() {
            out.push_str("\n      <agents>");
            for agent in &host.agents {
                let _ = write!(out, "\n        <agent ref=\"{}\"", attr(&agent.reference));
                if !agent.about.is_empty() {
                    let _ = write!(out, " about=\"{}\"", attr(&agent.about));
                }
                out.push_str(" />");
            }
            out.push_str("\n      </agents>");
        }
        out.push_str("\n    </host>");
    }
    out.push_str("\n  </hosts>");
}
fn render_channels(out: &mut String, workspaces: Option<&[WorkspaceView]>, now: u64) {
    let Some(workspaces) = workspaces else {
        return;
    };
    if !workspaces
        .iter()
        .any(|workspace| workspace.root.is_some() || !workspace.channels.is_empty())
    {
        return;
    }
    out.push_str("\n  <channels>");
    for workspace in workspaces {
        if let Some(root) = &workspace.root {
            render_channel(out, root, 4, now);
        }
        for channel in &workspace.channels {
            render_channel(out, channel, 4, now);
        }
    }
    out.push_str("\n  </channels>");
}
fn render_channel(out: &mut String, channel: &ChannelBlock, indent: usize, now: u64) {
    let pad = " ".repeat(indent);
    let name = attr(&channel.path);
    let _ = write!(out, "\n{pad}<channel name=\"{name}\"");
    if !channel.about.is_empty() {
        let _ = write!(out, " about=\"{}\"", attr(&channel.about));
    }
    if let Some(count) = channel.agent_count {
        let _ = write!(out, " agents=\"{count}\"");
    }
    if let Some(last_active) = &channel.last_active {
        let _ = write!(out, " last-active=\"{}\"", attr(last_active));
    }
    if channel.is_compact() {
        out.push_str(" />");
        return;
    }
    out.push('>');
    render_members(
        out,
        &channel.members,
        &channel.presence,
        &channel.departures,
        indent + 2,
    );
    render_messages(out, channel, indent + 2, now);
    for child in &channel.children {
        render_channel(out, child, indent + 2, now);
    }
    let _ = write!(out, "\n{pad}</channel>");
}
fn render_members(
    out: &mut String,
    members: &[MemberRow],
    presence: &[PresenceRow],
    departures: &[String],
    indent: usize,
) {
    if members.is_empty() && presence.is_empty() && departures.is_empty() {
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
        render_origin(out, &member.host, &member.workspace, &member.branch);
        if let Some(state) = member.state {
            let _ = write!(out, " state=\"{}\"", state.as_str());
        }
        if !member.status.is_empty() {
            let _ = write!(out, " status=\"{}\"", attr(&member.status));
        }
        if !member.since.is_empty() {
            let _ = write!(out, " since=\"{}\"", attr(&member.since));
        }
        out.push_str(" />");
    }
    for status in presence {
        let name = attr(status.name.trim_start_matches('@'));
        let state = status.state.as_str();
        let since = attr(&status.since);
        let _ = write!(out, "\n{child_pad}<agent name=\"@{name}\"");
        render_origin(out, &status.host, &status.workspace, &status.branch);
        let _ = write!(out, " state=\"{state}\"");
        if !status.status.is_empty() {
            let _ = write!(out, " status=\"{}\"", attr(&status.status));
        }
        let _ = write!(out, " since=\"{since}\" />");
        if let Some(failure) = &status.native_failure {
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
    for name in departures {
        let _ = write!(
            out,
            "\n{child_pad}@{} left.",
            text(name.trim_start_matches('@'))
        );
    }
    let _ = write!(out, "\n{pad}</members>");
}
fn render_origin(out: &mut String, host: &str, workspace: &str, branch: &str) {
    if !host.is_empty() {
        let _ = write!(out, " host=\"{}\"", attr(host));
    }
    if !workspace.is_empty() {
        let _ = write!(out, " workspace=\"{}\"", attr(workspace));
    }
    if !branch.is_empty() {
        let _ = write!(out, " branch=\"{}\"", attr(branch));
    }
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
