//! Prompt rendering for fabric message injection.
//!
//! Terminal-injected mentions use a structured envelope:
//!
//! ```text
//! <tenex-edge>
//!   <channel ref="workspace.channel.qa">
//!     <message from="@developer/session" id="abc123">hello</message>
//!   </channel>
//!
//!   Reply via: `tenex-edge channel reply abc123 --message "hello world"`
//! </tenex-edge>
//! ```
//!
//! Publishing no longer happens automatically on the agent's behalf — the
//! envelope carries an explicit reminder to respond via `tenex-edge channel
//! send`, since nothing mirrors the reply for it.
//!
//! Hook-delivered mentions and ambient channel activity are rendered by the
//! unified fabric context view, not by this envelope module.
//!
//! Echo suppression no longer lives in this text; direct delivery records the
//! pasted inbox event ids as explicit `injected` ledger rows. Envelopes are free
//! to be bare. Message ids are intentionally absent: replies target `@name`.

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
    lines.push("<tenex-edge>".to_string());
    for row in rows {
        lines.push(format!(
            "  <channel ref=\"{}\">",
            esc_attr(&crate::channel_ref::full_channel_ref(store, &row.channel_h))
        ));
        lines.push(format!(
            "    <message from=\"{}\" id=\"{}\">{}</message>",
            esc_attr(&format!("@{}", speaker_label(store, &row.from_pubkey))),
            esc_attr(&crate::util::short_id(&row.event_id)),
            esc_text(&row.body)
        ));
        lines.push("  </channel>".to_string());
    }
    if let Some(row) = rows.last() {
        let id = crate::util::short_id(&row.event_id);
        lines.push(String::new());
        lines.push(format!(
            "  Reply via: `tenex-edge channel reply {id} --message \"hello world\"`"
        ));
    }
    lines.push("</tenex-edge>".to_string());
    Some(lines.join("\n"))
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
