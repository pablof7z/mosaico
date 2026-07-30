use super::*;

pub(super) fn render_messages(out: &mut String, channel: &ChannelBlock, indent: usize, now: u64) {
    if channel.messages.is_empty() && channel.omitted == 0 {
        return;
    }
    let pad = " ".repeat(indent);
    let child_pad = " ".repeat(indent + 2);
    let _ = write!(out, "\n{pad}<chatter>");
    if channel.omitted > 0 {
        let _ = write!(
            out,
            "\n{child_pad}<omitted count=\"{}\" window=\"last 4h\" />",
            channel.omitted
        );
    }
    for message in &channel.messages {
        crate::agent_xml::write_message(
            out,
            indent + 2,
            &crate::agent_xml::MessageElement {
                event_id: &message.id,
                from: &message.from,
                recipients: &message.recipients,
                body: &message.body,
                created_at: message.created_at,
                now,
            },
        );
        if message.mention && message.needs_reply_nudge {
            let _ = write!(
                out,
                "\n{child_pad}Need a follow-up? Read \
                 `skills/mosaico/references/coordination-guide.md`."
            );
        }
    }
    let _ = write!(out, "\n{pad}</chatter>");
}
pub(super) fn render_important(out: &mut String, rows: &[ImportantRow]) {
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
pub(super) fn render_reactions(out: &mut String, rows: &[ReactionRow], omitted: usize) {
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
