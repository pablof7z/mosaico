use super::*;

pub(super) fn render_messages(out: &mut String, channel: &ChannelBlock, indent: usize) {
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
pub(super) fn render_mention_message(out: &mut String, message: &MessageRow, pad: &str) {
    let short = crate::util::short_id(&message.id);
    let from = attr(&message.from);
    let id = attr(&short);
    let body = text(&message.body);
    let _ = write!(
        out,
        "\n{pad}<message from=\"@{from}\" id=\"{id}\">{body}</message>"
    );
    if message.truncated {
        let _ = write!(
            out,
            "\n{pad}[message truncated; run `mosaico channel read --id {}`]",
            text(&short)
        );
    }
    if message.needs_reply_nudge {
        let _ = write!(
            out,
            "\n{pad}Need a follow-up? Read `skills/mosaico/references/coordination-guide.md`."
        );
    }
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
