use crate::fabric_context::model::*;
use std::fmt::Write as _;

pub(in crate::fabric_context) fn render_view(view: &FabricView) -> String {
    let mut out = String::from("<tenex-edge>");
    render_self(&mut out, view.self_row.as_ref());
    let _ = write!(
        out,
        "\n\n  <project name=\"{}\"",
        esc_attr(&view.project.name)
    );
    if !view.project.about.is_empty() {
        let _ = write!(out, " about=\"{}\"", esc_attr(&view.project.about));
    }
    out.push('>');
    render_agents(&mut out, &view.agents);
    for channel in &view.channels {
        render_channel(&mut out, channel);
    }
    render_unjoined(&mut out, &view.unjoined);
    out.push_str("\n  </project>");
    render_important(&mut out, &view.important);
    render_warnings(&mut out, &view.warnings);
    out.push_str("\n</tenex-edge>");
    out
}

fn render_self(out: &mut String, row: Option<&SelfRow>) {
    let Some(row) = row else {
        return;
    };
    let _ = write!(
        out,
        "\n  You are @{} on {} (session {}).",
        esc_text(&row.agent),
        esc_text(&row.backend),
        crate::util::friendly_short_code(&row.session_id)
    );
}

fn render_agents(out: &mut String, agents: &[AgentRow]) {
    if agents.is_empty() {
        return;
    }
    out.push_str("\n    <agents>");
    for a in agents {
        let _ = write!(out, "\n      <agent ref=\"@{}\"", esc_attr(&a.reference));
        if !a.about.is_empty() {
            let _ = write!(out, " about=\"{}\"", esc_attr(&a.about));
        }
        out.push_str(" />");
    }
    out.push_str("\n    </agents>");
}

fn render_channel(out: &mut String, channel: &ChannelBlock) {
    let _ = write!(
        out,
        "\n\n    <channel name=\"#{}\"",
        esc_attr(&channel.name)
    );
    if !channel.about.is_empty() {
        let _ = write!(out, " about=\"{}\"", esc_attr(&channel.about));
    }
    out.push('>');
    render_members(out, &channel.members);
    render_presence(out, &channel.presence);
    render_subchannels(out, &channel.subchannels);
    render_messages(out, channel);
    out.push_str("\n    </channel>");
}

fn render_members(out: &mut String, members: &[MemberRow]) {
    if members.is_empty() {
        return;
    }
    out.push_str("\n      <members>");
    for m in members {
        let _ = write!(
            out,
            "\n        <member ref=\"@{}\" status=\"{}\" seen=\"{}\" />",
            esc_attr(&m.reference),
            esc_attr(&m.status),
            esc_attr(&m.seen)
        );
    }
    out.push_str("\n      </members>");
}

fn render_presence(out: &mut String, presence: &[PresenceRow]) {
    if presence.is_empty() {
        return;
    }
    out.push_str("\n      <recent-presence>");
    for p in presence {
        let _ = write!(
            out,
            "\n        <status ref=\"@{}\" text=\"{}\" seen=\"{}\" />",
            esc_attr(&p.reference),
            esc_attr(&p.status),
            esc_attr(&p.seen)
        );
    }
    out.push_str("\n      </recent-presence>");
}

fn render_subchannels(out: &mut String, subs: &[ChannelSummaryRow]) {
    if subs.is_empty() {
        return;
    }
    out.push_str("\n      <subchannels>");
    for ch in subs {
        let _ = write!(out, "\n        <channel name=\"#{}\"", esc_attr(&ch.name));
        if !ch.about.is_empty() {
            let _ = write!(out, " about=\"{}\"", esc_attr(&ch.about));
        }
        out.push_str(" />");
    }
    out.push_str("\n      </subchannels>");
}

fn render_messages(out: &mut String, channel: &ChannelBlock) {
    if channel.messages.is_empty() && channel.omitted == 0 {
        return;
    }
    out.push_str("\n      <chatter>");
    if channel.omitted > 0 {
        let _ = write!(
            out,
            "\n        <omitted count=\"{}\" window=\"last 4h\" />",
            channel.omitted
        );
    }
    for m in &channel.messages {
        out.push_str("\n        <message");
        let short = crate::util::short_id(&m.id);
        if m.truncated {
            let _ = write!(out, " id=\"{}\"", esc_attr(&short));
        }
        let _ = write!(
            out,
            " from=\"@{}\" age=\"{}\">",
            esc_attr(&m.from),
            esc_attr(&m.age)
        );
        if m.mention {
            out.push_str("[MENTIONS YOU] ");
        }
        out.push_str(&esc_text(&m.body));
        if m.truncated {
            let _ = write!(
                out,
                "\n          [message truncated; run `tenex-edge chat read --id {}`]",
                esc_text(&short)
            );
        }
        if m.mention {
            out.push_str(
                "\n          [reply via `tenex-edge chat write --message \"...\"` \
                 — replies do not auto-publish]",
            );
        }
        out.push_str("</message>");
    }
    out.push_str("\n      </chatter>");
}

fn render_unjoined(out: &mut String, unjoined: &[UnjoinedChannelRow]) {
    if unjoined.is_empty() {
        return;
    }
    out.push_str("\n\n    <channels-not-joined>");
    for ch in unjoined {
        let _ = write!(
            out,
            "\n      <channel name=\"#{}\" last_active=\"{}\"",
            esc_attr(&ch.name),
            esc_attr(&ch.last_active)
        );
        if !ch.about.is_empty() {
            let _ = write!(out, " about=\"{}\"", esc_attr(&ch.about));
        }
        out.push_str(" />");
    }
    out.push_str("\n    </channels-not-joined>");
}

fn render_important(out: &mut String, rows: &[ImportantRow]) {
    if rows.is_empty() {
        return;
    }
    out.push_str("\n\n  <important>");
    for row in rows {
        let _ = write!(
            out,
            "\n    <mention channel=\"#{}\" message_id=\"{}\" />",
            esc_attr(&row.channel),
            esc_attr(&crate::util::short_id(&row.message_id))
        );
    }
    out.push_str("\n  </important>");
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

fn esc_attr(input: &str) -> String {
    esc_text(input).replace('"', "&quot;")
}

fn esc_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
