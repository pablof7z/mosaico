//! Prompt rendering for fabric message injection.
//!
//! Two tmux envelope forms, chosen by `(sender, directedness)`:
//!
//!   1. **tmux + human mention** — pasted into the pane as a real turn, minimal
//!      provenance: `<@pablo> @developer hey there`. The agent's reply is
//!      auto-captured and published, so no reply instructions are needed.
//!   2. **tmux + agent mention** — pasted as a turn, framed so the agent knows it
//!      arrived via the fabric: `[tenex-edge mention] <@agent1> hi @developer`.
//!
//! Hook-delivered mentions and ambient channel activity are rendered by the
//! unified fabric context view, not by this envelope module.
//!
//! Echo suppression no longer lives in this text (it moved to
//! [`crate::daemon::server`]'s per-session echo guard), so envelopes are free to
//! be bare. Message ids are intentionally absent: replies target `@name`.

use crate::state::{InboxRow, Store};
use crate::util::pubkey_short;

fn speaker_chip(name: &str) -> String {
    format!("<@{name}>")
}

/// Display name for a pubkey: its cached `kind:0` slug, else a short hex form.
fn speaker_label(store: &Store, pubkey: &str) -> String {
    store
        .resolve_slug_for_pubkey(pubkey)
        .ok()
        .flatten()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| pubkey_short(pubkey))
}

/// True when `pubkey` is one of the operator's whitelisted humans, i.e. the
/// mention came from a person rather than another agent.
fn is_whitelisted(whitelisted: &[String], pubkey: &str) -> bool {
    whitelisted.iter().any(|w| w.eq_ignore_ascii_case(pubkey))
}

/// Form ① / ② — direct mentions pasted into a live tmux pane as a real turn.
/// Human senders render bare with a `<@name>` prefix (it reads as a near-natural
/// turn that still carries provenance); agent senders are prefixed
/// `[tenex-edge mention]` so the agent knows it is a fabric relay, not its
/// operator typing. No reply hint, no message id — the reply auto-publishes.
pub(crate) fn render_tmux_mention(
    store: &Store,
    rows: &[InboxRow],
    whitelisted: &[String],
    _now: u64,
) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = Vec::with_capacity(rows.len());
    for row in rows {
        let name = speaker_label(store, &row.from_pubkey);
        let chip = speaker_chip(&name);
        if is_whitelisted(whitelisted, &row.from_pubkey) {
            lines.push(format!("{chip} {}", row.body));
        } else {
            lines.push(format!("[tenex-edge mention] {chip} {}", row.body));
        }
    }
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
