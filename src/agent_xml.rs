//! Shared XML primitives for agent-facing Mosaico documents.
//!
//! [`write_message`] is the sole production serializer for `<message>` nodes.
//! Callers select and group messages, but do not reproduce message attributes,
//! body truncation, escaping, or recovery guidance.

use std::fmt::Write as _;

use crate::util::{short_id, truncate_words, CHAT_RENDER_WORD_LIMIT};

pub(crate) struct MessageElement<'a> {
    pub(crate) event_id: &'a str,
    pub(crate) from: &'a str,
    pub(crate) recipients: &'a [String],
    pub(crate) attachment_dir: &'a str,
    pub(crate) body: &'a str,
    pub(crate) created_at: u64,
    pub(crate) now: u64,
}

/// Append one indented canonical `<message>` element.
pub(crate) fn write_message(out: &mut String, indent: usize, message: &MessageElement<'_>) {
    let pad = " ".repeat(indent);
    let detail_pad = " ".repeat(indent + 2);
    let id = short_id(message.event_id);
    let from = attr(message.from.trim_start_matches('@'));
    let _ = write!(out, "\n{pad}<message from=\"@{from}\" id=\"{}\"", attr(&id));
    if !message.recipients.is_empty() {
        let recipients = message
            .recipients
            .iter()
            .map(|recipient| format!("@{}", recipient.trim_start_matches('@')))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(out, " for=\"{}\"", attr(&recipients));
    }
    if !message.attachment_dir.is_empty() {
        let _ = write!(out, " attachment-dir=\"{}\"", attr(message.attachment_dir));
    }
    write_time(out, message.created_at, message.now);

    let (body, truncated) = truncate_words(message.body, CHAT_RENDER_WORD_LIMIT);
    let _ = write!(out, ">{}", text(&body));
    if truncated {
        let _ = write!(
            out,
            "\n{detail_pad}[message truncated; run `mosaico channel read --id {}`]",
            text(&id)
        );
    }
    out.push_str("</message>");
}

fn write_time(out: &mut String, created_at: u64, now: u64) {
    let elapsed = now.saturating_sub(created_at);
    if elapsed <= 3_600 {
        let age = if elapsed < 60 {
            format!("{elapsed}s")
        } else if elapsed < 3_600 {
            format!("{}m", elapsed / 60)
        } else {
            "1h".to_string()
        };
        let _ = write!(out, " age=\"{age}\"");
    } else {
        let _ = write!(out, " time=\"{created_at}\"");
    }
}

pub(crate) fn attr(input: &str) -> String {
    text(input).replace('"', "&quot;").replace('\'', "&apos;")
}

pub(crate) fn text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(created_at: u64, now: u64) -> String {
        let mut out = String::new();
        write_message(
            &mut out,
            2,
            &MessageElement {
                event_id: "abcdef123456",
                from: "@Pablo",
                recipients: &["reviewer".to_string()],
                attachment_dir: "",
                body: "ship it",
                created_at,
                now,
            },
        );
        out
    }

    #[test]
    fn recent_age_is_compact_through_the_one_hour_boundary() {
        assert!(render(1_000, 1_030).contains(" age=\"30s\""));
        assert!(render(1_000, 1_060).contains(" age=\"1m\""));
        let one_hour = render(1_000, 4_600);
        assert!(one_hour.contains(" age=\"1h\""), "{one_hour}");
        assert!(!one_hour.contains(" time="), "{one_hour}");
    }

    #[test]
    fn older_messages_use_only_absolute_unix_time() {
        let older = render(1_000, 4_601);
        assert!(older.contains(" time=\"1000\""), "{older}");
        assert!(!older.contains(" age="), "{older}");
    }

    #[test]
    fn message_contract_escapes_and_recovers_with_the_short_id() {
        let body = (0..=CHAT_RENDER_WORD_LIMIT)
            .map(|index| {
                if index == 0 {
                    "<first&>".to_string()
                } else {
                    format!("word{index}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let mut out = String::new();
        write_message(
            &mut out,
            4,
            &MessageElement {
                event_id: "abcdef123456",
                from: "Pablo\"",
                recipients: &["reviewer&one".to_string(), "@chief".to_string()],
                attachment_dir: "/tmp/mosaico files/abcdef",
                body: &body,
                created_at: 1_000,
                now: 5_000,
            },
        );

        assert!(out.contains("from=\"@Pablo&quot;\" id=\"abcdef\""));
        assert!(out.contains("for=\"@reviewer&amp;one @chief\""));
        assert!(out.contains("attachment-dir=\"/tmp/mosaico files/abcdef\""));
        assert!(out.contains("&lt;first&amp;&gt;"));
        assert!(out.contains("time=\"1000\""));
        assert!(out.contains("mosaico channel read --id abcdef"));
        assert!(!out.contains("word100</message>"), "{out}");
    }
}
