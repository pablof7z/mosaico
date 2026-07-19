use crate::fabric_context::model::*;
use std::fmt::Write as _;
pub(in crate::fabric_context) mod all_workspaces;
mod workspace;
use workspace::render_workspace_block;
pub(in crate::fabric_context) fn render_view(view: &FabricView) -> String {
    let mut out = String::from("<mosaico>");
    render_self(&mut out, view.self_row.as_ref());
    render_workspace(&mut out, view);
    out.push_str("\n</mosaico>");
    out
}

pub(super) fn render_workspace(out: &mut String, view: &FabricView) {
    if view.is_quiet_delta() {
        render_no_new_activity(out, &view.workspace.name);
        return;
    }
    render_workspace_block(out, &view.workspace, view.root.as_ref(), &view.channels);
    for activity in &view.other_workspaces {
        render_workspace_block(
            out,
            &activity.workspace,
            activity.root.as_ref(),
            &activity.channels,
        );
    }
    render_important(out, &view.important);
    render_reactions(out, &view.reactions, view.reactions_omitted);
    render_warnings(out, &view.warnings);
}

fn render_no_new_activity(out: &mut String, workspace: &str) {
    let _ = write!(
        out,
        "\n\n  <no-new-activity workspace=\"{}\">\
         \n    Nothing new since your last check. The fabric surfaces only what \
         changed — your channels, members, and messages are unchanged, not gone.\
         \n  </no-new-activity>",
        esc_attr(workspace)
    );
}

fn render_self(out: &mut String, row: Option<&SelfRow>) {
    let Some(row) = row else {
        return;
    };
    let _ = write!(
        out,
        "\n  Agent: {} · Session: @{} · Backend: {}",
        esc_text(&row.agent_slug),
        esc_text(&row.agent),
        esc_text(&row.host)
    );
    if !row.title.is_empty() {
        let _ = write!(
            out,
            "\n  Current title: \"{}\"\n  [if your title drifted you can update it]",
            esc_text(&row.title)
        );
    }
}

pub(super) fn render_channel(out: &mut String, channel: &ChannelBlock, indent: usize) {
    let pad = " ".repeat(indent);
    let _ = write!(
        out,
        "\n{pad}<channel name=\"#{}\" ref=\"{}\"",
        esc_attr(&channel.name),
        esc_attr(&channel.reference)
    );
    if !channel.about.is_empty() {
        let _ = write!(out, " about=\"{}\"", esc_attr(&channel.about));
    }
    if channel.is_compact() {
        out.push_str(" />");
        return;
    }
    out.push('>');
    render_channel_body(out, channel, indent + 2);
    for child in &channel.children {
        render_channel(out, child, indent + 2);
    }
    let _ = write!(out, "\n{pad}</channel>");
}

pub(super) fn render_channel_body(out: &mut String, channel: &ChannelBlock, indent: usize) {
    render_members(out, &channel.members, indent);
    render_presence(out, &channel.presence, indent);
    render_messages(out, channel, indent);
}

fn render_members(out: &mut String, members: &[MemberRow], indent: usize) {
    if members.is_empty() {
        return;
    }
    let pad = " ".repeat(indent);
    let child_pad = " ".repeat(indent + 2);
    let _ = write!(out, "\n{pad}<members>");
    for member in members {
        let _ = write!(
            out,
            "\n{child_pad}<member ref=\"@{}\" state=\"{}\" status=\"{}\" seen=\"{}\" />",
            esc_attr(&member.reference),
            member.state.as_str(),
            esc_attr(&member.status),
            esc_attr(&member.seen)
        );
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
        let _ = write!(
            out,
            "\n{child_pad}<status ref=\"@{}\" state=\"{}\" text=\"{}\" seen=\"{}\" />",
            esc_attr(&status.reference),
            status.state.as_str(),
            esc_attr(&status.status),
            esc_attr(&status.seen)
        );
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
        let _ = write!(out, "\n{child_pad}<message");
        let short = crate::util::short_id(&message.id);
        if message.truncated {
            let _ = write!(out, " id=\"{}\"", esc_attr(&short));
        }
        let _ = write!(out, " from=\"@{}\"", esc_attr(&message.from));
        if !message.recipients.is_empty() {
            let recipients = message
                .recipients
                .iter()
                .map(|recipient| format!("@{recipient}"))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = write!(out, " for=\"{}\"", esc_attr(&recipients));
        }
        let _ = write!(out, " age=\"{}\">", esc_attr(&message.age));
        out.push_str(&esc_text(&message.body));
        if message.truncated {
            let _ = write!(
                out,
                "\n{detail_pad}[message truncated; run `mosaico channel read --id {}`]",
                esc_text(&short)
            );
        }
        out.push_str("</message>");
    }
    let _ = write!(out, "\n{pad}</chatter>");
}

fn render_mention_message(out: &mut String, message: &MessageRow, pad: &str) {
    let short = crate::util::short_id(&message.id);
    let _ = write!(
        out,
        "\n{pad}<message from=\"@{}\" id=\"{}\">{}</message>",
        esc_attr(&message.from),
        esc_attr(&short),
        esc_text(&message.body)
    );
    let _ = write!(
        out,
        "\n{pad}Reply via: `mosaico channel reply {} --message \"hello world\"`",
        esc_text(&short)
    );
    let _ = write!(out, "\n{pad}{}", crate::attachment::AGENT_HINT);
    let _ = write!(
        out,
        "\n{pad}Ack-only? `mosaico channel react {} 👍` — never interrupts. \
         Reply only with substantive content; never send a bare \"ok\"/\"noted\".",
        esc_text(&short)
    );
}

fn render_important(out: &mut String, rows: &[ImportantRow]) {
    if rows.is_empty() {
        return;
    }
    out.push_str("\n\n  <important>");
    for row in rows {
        let _ = write!(
            out,
            "\n    <mention channel=\"{}\" message_id=\"{}\" />",
            esc_attr(&row.channel_ref),
            esc_attr(&crate::util::short_id(&row.message_id))
        );
    }
    out.push_str("\n  </important>");
}

fn render_reactions(out: &mut String, rows: &[ReactionRow], omitted: usize) {
    if rows.is_empty() {
        return;
    }
    out.push_str("\n\n  <reactions>");
    for row in rows {
        let reactors = row
            .reactors
            .iter()
            .map(|r| format!("@{}", esc_text(r)))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(
            out,
            "\n    {} {} on your message \"{}\" ({})",
            reactors,
            esc_text(&row.emoji),
            esc_text(&row.target_snippet),
            esc_text(&row.age)
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
    out.push_str("\n\n  <warnings>");
    for row in rows {
        let _ = write!(out, "\n    <warning>{}</warning>", esc_text(&row.text));
    }
    out.push_str("\n  </warnings>");
}

pub(super) fn esc_attr(input: &str) -> String {
    esc_text(input).replace('"', "&quot;")
}

fn esc_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
