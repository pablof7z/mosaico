//! Prompt rendering for fabric message injection.
//!
//! Terminal-injected mentions use a structured envelope:
//!
//! ```text
//! <mosaico>
//!   <channel ref="/workspace/channel/qa">
//!     <message from="@mist-ridge-204-developer" id="abc123">hello</message>
//!   </channel>
//! </mosaico>
//! ```
//!
//! Publishing no longer happens automatically on the agent's behalf — when the
//! target has not already been answered or acknowledged, the envelope carries a
//! brief reminder to consult `skills/mosaico/references/coordination-guide.md`.
//!
//! Hook-delivered mentions and ambient channel activity are rendered by the
//! unified fabric context view, not by this envelope module.
//!
//! Echo suppression no longer lives in this text; direct delivery records the
//! pasted inbox event ids as explicit `injected` ledger rows. Envelopes are free
//! to be bare. Message ids are always present so agents can reply or react to
//! the exact message.

use crate::state::{InboxRow, Store};
use crate::util::pubkey_short;

/// Display name for a pubkey: its cached `kind:0` slug, else a short hex form.
fn speaker_label(store: &Store, pubkey: &str) -> String {
    store
        .resolve_slug_for_pubkey(pubkey)
        .ok()
        .flatten()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| pubkey_short(pubkey))
}

/// Direct mentions submitted into a live terminal as a real turn.
pub(crate) fn render_terminal_mention(
    store: &Store,
    rows: &[InboxRow],
    _whitelisted: &[String],
    _now: u64,
) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = Vec::with_capacity(rows.len() * 3 + 4);
    lines.push("<mosaico>".to_string());
    for row in rows {
        push_agent_message(
            &mut lines,
            &crate::channel_ref::full_channel_ref(store, &row.channel_h),
            &speaker_label(store, &row.from_pubkey),
            &row.event_id,
            &row.body,
        );
    }
    if let Some(row) = rows.last() {
        if should_render_reply_nudge(store, row) {
            lines.push(String::new());
            push_reply_hint(&mut lines);
        }
    }
    lines.push("</mosaico>".to_string());
    Some(lines.join("\n"))
}

/// Render one blocking-wait success in the same agent-native envelope used for
/// direct terminal delivery. The wait command intentionally has no alternate
/// human or JSON renderer.
pub(crate) fn render_agent_message(
    channel_ref: &str,
    from: &str,
    event_id: &str,
    body: &str,
) -> String {
    let mut lines = vec!["<mosaico>".to_string()];
    push_agent_message(&mut lines, channel_ref, from, event_id, body);
    lines.push("</mosaico>".to_string());
    lines.join("\n")
}

/// Render the expected timeout outcome without switching to a second output
/// convention. Channel refs document the exact start-time scope snapshot.
pub(crate) fn render_agent_wait_timeout(seconds: u64, channels: &[&str]) -> String {
    let mut lines = vec![
        "<mosaico>".to_string(),
        format!("  <wait outcome=\"timeout\" after=\"{seconds}s\">"),
    ];
    lines.extend(
        channels
            .iter()
            .map(|channel| format!("    <channel ref=\"{}\" />", esc_attr(channel))),
    );
    lines.push("  </wait>".to_string());
    lines.push("</mosaico>".to_string());
    lines.join("\n")
}

fn push_agent_message(
    lines: &mut Vec<String>,
    channel_ref: &str,
    from: &str,
    event_id: &str,
    body: &str,
) {
    lines.push(format!("  <channel ref=\"{}\">", esc_attr(channel_ref)));
    lines.push(format!(
        "    <message from=\"@{}\" id=\"{}\">{}</message>",
        esc_attr(from.trim_start_matches('@')),
        esc_attr(&crate::util::short_id(event_id)),
        esc_text(body)
    ));
    lines.push("  </channel>".to_string());
}

fn push_reply_hint(lines: &mut Vec<String>) {
    lines.push(
        "  Need a follow-up? Read `skills/mosaico/references/coordination-guide.md`.".to_string(),
    );
}

fn should_render_reply_nudge(store: &Store, row: &InboxRow) -> bool {
    store
        .should_render_reply_nudge(
            &row.channel_h,
            &row.event_id,
            &row.target_pubkey,
            row.created_at,
        )
        .unwrap_or(true)
}

/// The human display label for a channel: its kind:39000 `name` when set, else
/// the raw `channel_h` as a genuine fallback. The opaque id must never appear in
/// agent-facing text when a name exists.
pub(crate) fn channel_display(store: &Store, channel_h: &str) -> String {
    store
        .get_channel(channel_h)
        .ok()
        .flatten()
        .and_then(|c| c.human_name().map(str::to_string))
        .unwrap_or_else(|| channel_h.to_string())
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
