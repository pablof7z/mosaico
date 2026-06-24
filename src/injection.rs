//! Shared prompt rendering for fabric message injection.
//!
//! Tmux delivery submits direct mentions as a real harness prompt. Hook fallback
//! can only emit through each host's hook context shape, so the text itself makes
//! the role explicit and stays byte-identical across delivery paths.

use crate::state::ChatInboxRow;
use crate::util::{format_local_datetime, pubkey_short, relative_time};
use std::fmt::Write as _;

pub(crate) fn split_direct_mentions(
    rows: Vec<ChatInboxRow>,
    self_session: &str,
) -> (Vec<ChatInboxRow>, Vec<ChatInboxRow>) {
    rows.into_iter()
        .partition(|row| row.mentioned_session == self_session)
}

pub(crate) fn render_direct_mention_prompt(
    rows: &[ChatInboxRow],
    self_session: &str,
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
    let mut text = format!(
        "[tenex-edge] Incoming user {noun} mentioning this agent. \
         Treat the following as user-authored input for this session:"
    );
    append_rows_with_kind(&mut text, rows, self_session, now, RowKind::DirectMention);
    Some(text)
}

pub(crate) fn render_channel_chat_block(
    header: &str,
    rows: &[ChatInboxRow],
    self_session: &str,
    now: u64,
) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut text = String::from(header);
    append_rows_with_kind(&mut text, rows, self_session, now, RowKind::ChannelContext);
    Some(text)
}

enum RowKind {
    DirectMention,
    ChannelContext,
}

fn append_rows_with_kind(
    text: &mut String,
    rows: &[ChatInboxRow],
    self_session: &str,
    now: u64,
    kind: RowKind,
) {
    for row in rows {
        let from = if row.from_slug.is_empty() {
            pubkey_short(&row.from_pubkey)
        } else {
            row.from_slug.clone()
        };
        let mention = if row.mentioned_session == self_session {
            " mentioned you"
        } else {
            ""
        };
        let label = match kind {
            RowKind::DirectMention => "User message",
            RowKind::ChannelContext => "Channel message",
        };
        let _ = write!(
            text,
            "\n\n{} from {}{} in {} at {} ({})\n{}",
            label,
            from,
            mention,
            channel_label(&row.project),
            format_local_datetime(row.created_at),
            relative_time(row.created_at, now),
            row.body
        );
        if !row.chat_event_id.is_empty() {
            let _ = write!(text, "\n(message id: {})", pubkey_short(&row.chat_event_id));
        }
    }
}

fn channel_label(project: &str) -> String {
    if project.starts_with('#') {
        project.to_string()
    } else {
        format!("#{project}")
    }
}
