//! Shared prompt rendering for fabric message injection.
//!
//! Tmux delivery submits direct mentions as a real harness prompt. Hook fallback
//! can only emit through each host's hook context shape, so the text itself makes
//! the role explicit and stays byte-identical across delivery paths.

use crate::state::{InboxRow, RelayEvent, Store};
use crate::util::{format_local_datetime, pubkey_short, relative_time};
use std::fmt::Write as _;

/// Prefix every fabric-injected prompt carries. The daemon pastes these envelopes
/// into a pane as a real harness prompt; the resulting `user-prompt-submit` hook
/// must NOT republish them (they are already kind:9 events in the room). A human
/// could in principle type a prompt starting with this tool-namespaced marker,
/// but that is vanishingly rare and only costs the echo of one prompt — an
/// acceptable trade for breaking the injection echo loop without per-message
/// bookkeeping.
pub(crate) const FABRIC_INJECTION_MARKER: &str = "[tenex-edge]";

/// True when `prompt` is a daemon-injected fabric envelope rather than human
/// keyboard input — i.e. content that is already published and must not be
/// mirrored back into the room by the user-prompt publish path.
pub(crate) fn is_fabric_injection(prompt: &str) -> bool {
    prompt.trim_start().starts_with(FABRIC_INJECTION_MARKER)
}

/// The render-relevant projection of any inbound row. Both the inbox ledger
/// (direct mentions) and the verbatim relay-event log (ambient channel chat)
/// flatten to this shape so a single renderer serves both paths.
struct RenderRow<'a> {
    sender_pubkey: &'a str,
    channel_h: &'a str,
    body: &'a str,
    created_at: u64,
    event_id: &'a str,
}

impl<'a> From<&'a InboxRow> for RenderRow<'a> {
    fn from(r: &'a InboxRow) -> Self {
        RenderRow {
            sender_pubkey: &r.from_pubkey,
            channel_h: &r.channel_h,
            body: &r.body,
            created_at: r.created_at,
            event_id: &r.event_id,
        }
    }
}

impl<'a> From<&'a RelayEvent> for RenderRow<'a> {
    fn from(r: &'a RelayEvent) -> Self {
        RenderRow {
            sender_pubkey: &r.pubkey,
            channel_h: &r.channel_h,
            body: &r.content,
            created_at: r.created_at,
            event_id: &r.id,
        }
    }
}

pub(crate) fn render_direct_mention_prompt(
    store: &Store,
    rows: &[InboxRow],
    now: u64,
) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let noun = if rows.len() == 1 {
        "message"
    } else {
        "messages"
    };
    // Sender-agnostic preamble: a mention may originate from a human OR another
    // agent, so the envelope must not assert "user-authored".
    let mut text = format!(
        "{FABRIC_INJECTION_MARKER} Incoming {noun} mentioning this agent. \
         Treat the following as input addressed to you in this session:"
    );
    let render: Vec<RenderRow> = rows.iter().map(RenderRow::from).collect();
    append_rows_with_kind(store, &mut text, &render, now, RowKind::DirectMention);
    Some(text)
}

pub(crate) fn render_channel_chat_block(
    store: &Store,
    header: &str,
    rows: &[RelayEvent],
    now: u64,
) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut text = String::from(header);
    let render: Vec<RenderRow> = rows.iter().map(RenderRow::from).collect();
    append_rows_with_kind(store, &mut text, &render, now, RowKind::ChannelContext);
    Some(text)
}

enum RowKind {
    DirectMention,
    ChannelContext,
}

fn append_rows_with_kind(
    store: &Store,
    text: &mut String,
    rows: &[RenderRow],
    now: u64,
    kind: RowKind,
) {
    for row in rows {
        // Sender slug is no longer carried on the row; show the short pubkey. Body
        // `nostr:` mentions are rewritten to `@name` by the caller before render.
        let from = pubkey_short(row.sender_pubkey);
        // Sender-agnostic wording: a mention may come from a human OR another
        // agent, so never assume "user". A direct mention reads "Mention in
        // <channel> from <sender>"; sibling channel context stays "Channel
        // message in <channel> from <sender>". The channel is shown by its human
        // NAME (never the raw opaque id).
        let label = match kind {
            RowKind::DirectMention => {
                format!("Mention in {}", channel_label(store, row.channel_h))
            }
            RowKind::ChannelContext => {
                format!("Channel message in {}", channel_label(store, row.channel_h))
            }
        };
        let _ = write!(
            text,
            "\n\n{} from {} at {} ({})\n{}",
            label,
            from,
            format_local_datetime(row.created_at),
            relative_time(row.created_at, now),
            row.body
        );
        if !row.event_id.is_empty() {
            let _ = write!(text, "\n(message id: {})", pubkey_short(row.event_id));
        }
    }
}

/// The human display label for a channel: its kind:39000 `name` when set, else
/// the raw `channel_h` as a genuine fallback. The opaque id must never appear in
/// agent-facing text when a name exists.
pub(crate) fn channel_display(store: &Store, channel_h: &str) -> String {
    store
        .get_channel(channel_h)
        .ok()
        .flatten()
        .map(|c| c.name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| channel_h.to_string())
}

fn channel_label(store: &Store, channel_h: &str) -> String {
    channel_display(store, channel_h)
}
