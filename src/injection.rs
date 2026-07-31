//! Prompt rendering for fabric message injection.
//!
//! Terminal-injected mentions use a structured envelope:
//!
//! ```text
//! <mosaico>
//!   <channel ref="#workspace/channel/qa">
//!     <message from="@mist-ridge-204-developer" id="abc123"
//!              age="1m">hello</message>
//!   </channel>
//! </mosaico>
//! ```
//!
//! Direct injection never emits `for=…`: the envelope is already delivered
//! into the target session, so naming that recipient is redundant noise.
//!
//! Publishing no longer happens automatically on the agent's behalf — when the
//! target has not already been answered or acknowledged, the envelope carries a
//! compact reply/react affordance and, when its per-session cooldown allows, a
//! reminder pointing to the installed coordination guide.
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
use std::fmt::Write as _;

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
    now: u64,
    show_coordination_guide: bool,
) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut out = String::from("<mosaico>");
    for row in rows {
        let channel_ref = crate::channel_ref::full_channel_ref(store, &row.channel_h);
        if channel_ref.is_empty() {
            return None;
        }
        let from = speaker_label(store, &row.from_pubkey);
        let _ = write!(
            out,
            "\n  <channel ref=\"{}\">",
            crate::agent_xml::attr(&channel_ref)
        );
        // Empty recipients: this envelope is already being injected into the
        // target session, so a `for=` attribute would only restate the
        // delivery target.
        crate::agent_xml::write_message(
            &mut out,
            4,
            &crate::agent_xml::MessageElement {
                event_id: &row.event_id,
                from: &from,
                recipients: &[],
                attachment_dir: &row.attachment_dir,
                body: &row.body,
                created_at: row.created_at,
                now,
            },
        );
        if should_render_reply_nudge(store, row) {
            let _ = write!(
                out,
                "\n    Follow up on {}: reply for substantive context or react for an ACK.",
                crate::util::short_id(&row.event_id)
            );
        }
        out.push_str("\n  </channel>");
    }
    if show_coordination_guide {
        let _ = write!(
            out,
            "\n  <notice>{}</notice>",
            crate::agent_xml::text(crate::reconcile::COORDINATION_GUIDE_REMINDER)
        );
    }
    out.push_str("\n</mosaico>");
    Some(out)
}

/// Render the expected timeout outcome without switching to a second output
/// convention. Channel refs document the exact start-time scope snapshot.
pub(crate) fn render_agent_wait_timeout(seconds: u64, channels: &[&str]) -> String {
    let mut lines = vec![
        "<mosaico>".to_string(),
        format!("  <wait outcome=\"timeout\" after=\"{seconds}s\">"),
    ];
    lines.extend(channels.iter().map(|channel| {
        format!(
            "    <channel ref=\"{}\" />",
            crate::agent_xml::attr(channel)
        )
    }));
    lines.push("  </wait>".to_string());
    lines.push("</mosaico>".to_string());
    lines.join("\n")
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

pub(crate) fn has_unresolved_terminal_mention(store: &Store, rows: &[InboxRow]) -> bool {
    rows.iter().any(|row| should_render_reply_nudge(store, row))
}
