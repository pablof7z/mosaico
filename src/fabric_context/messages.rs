use super::model::MessageRow;
use super::refs::pubkey_ref;
use super::{FabricContextInput, FabricMessageSeed};
use crate::state::{RelayEvent, Store};
use crate::util::{relative_time, truncate_words, CHAT_RENDER_WORD_LIMIT};
use std::collections::HashSet;

const WINDOW_SECS: u64 = 4 * 60 * 60;
const CHAT_CAP: u32 = 10_000;
const MAX_CLUSTER_GAP_SECS: u64 = 20 * 60;
const MAX_CLUSTER_ROWS: usize = 30;

pub(super) fn message_rows(
    store: &Store,
    input: &FabricContextInput<'_>,
    channel: &str,
    forced: &[FabricMessageSeed],
) -> (Vec<MessageRow>, usize) {
    let since = if input.cursor == 0 {
        input.now.saturating_sub(WINDOW_SECS)
    } else {
        input.cursor
    };
    let mut events = store
        .chat_for_channel(channel, since, CHAT_CAP)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.kind == crate::fabric::nip29::wire::KIND_CHAT as u32)
        .filter(|e| e.pubkey != input.self_pubkey)
        .filter(|e| !is_backend_traffic(store, input.backend_pubkey, &e.pubkey, &e.tags_json))
        .collect::<Vec<_>>();
    // Messages already pasted verbatim into the pane (e.g. the mention that
    // spawned this turn) would otherwise also show up here, duplicating the
    // same text the agent already saw as literal prompt input.
    if let Some(session) = input.session {
        let injected: HashSet<String> = store
            .injected_for_pubkey(&session.pubkey)
            .unwrap_or_default()
            .into_iter()
            .map(|row| row.event_id)
            .collect();
        if !injected.is_empty() {
            events.retain(|e| !injected.contains(&e.id));
        }
    }
    let omitted = if input.cursor == 0 {
        let total = events.len();
        events = recent_cluster(events);
        total.saturating_sub(events.len())
    } else {
        0
    };
    let mut seen: HashSet<String> = events.iter().map(|e| e.id.clone()).collect();
    for row in forced {
        if seen.insert(row.id.clone()) {
            events.push(RelayEvent {
                id: row.id.clone(),
                kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
                pubkey: row.from_pubkey.clone(),
                created_at: row.created_at,
                channel_h: channel.to_string(),
                d_tag: String::new(),
                content: row.body.clone(),
                tags_json: forced_tags_json(input.self_pubkey, row.mention),
            });
        }
    }
    events.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let rows = events
        .into_iter()
        .map(|ev| {
            let mention = forced.iter().any(|f| f.id == ev.id && f.mention)
                || mentions_pubkey(&ev.tags_json, input.self_pubkey);
            message_row(store, &ev, input.now, input.local_host, mention)
        })
        .collect();
    (rows, omitted)
}

fn recent_cluster(mut events: Vec<RelayEvent>) -> Vec<RelayEvent> {
    if events.len() <= MAX_CLUSTER_ROWS {
        return events;
    }
    events.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let mut start = events.len().saturating_sub(1);
    while start > 0 {
        let gap = events[start]
            .created_at
            .saturating_sub(events[start - 1].created_at);
        if gap > MAX_CLUSTER_GAP_SECS || events.len() - start >= MAX_CLUSTER_ROWS {
            break;
        }
        start -= 1;
    }
    events.split_off(start)
}

fn message_row(
    store: &Store,
    ev: &RelayEvent,
    now: u64,
    local_host: &str,
    mention: bool,
) -> MessageRow {
    let resolved_body = crate::profile::rewrite_body_mentions(store, &ev.content);
    let (body, truncated) = truncate_words(&resolved_body, CHAT_RENDER_WORD_LIMIT);
    MessageRow {
        id: ev.id.clone(),
        channel_ref: crate::channel_ref::full_channel_ref(store, &ev.channel_h),
        from: pubkey_ref(store, &ev.pubkey, local_host),
        recipients: p_tag_refs(store, &ev.tags_json, local_host),
        age: relative_time(ev.created_at, now),
        body,
        mention,
        truncated,
    }
}

fn forced_tags_json(self_pubkey: &str, mention: bool) -> String {
    if !mention || self_pubkey.is_empty() {
        return "[]".to_string();
    }
    serde_json::to_string(&vec![vec!["p".to_string(), self_pubkey.to_string()]])
        .unwrap_or_else(|_| "[]".to_string())
}

fn p_tag_refs(store: &Store, tags_json: &str, local_host: &str) -> Vec<String> {
    p_tag_pubkeys(tags_json)
        .into_iter()
        .map(|pk| pubkey_ref(store, &pk, local_host))
        .collect()
}

pub(crate) fn p_tag_pubkeys(tags_json: &str) -> Vec<String> {
    let Ok(tags) = serde_json::from_str::<Vec<Vec<String>>>(tags_json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for tag in tags {
        if tag.first().is_some_and(|t| t == "p") {
            if let Some(pubkey) = tag.get(1).filter(|p| !p.is_empty()) {
                if !out.iter().any(|seen| seen == pubkey) {
                    out.push(pubkey.clone());
                }
            }
        }
    }
    out
}

pub(super) fn mentions_pubkey(tags_json: &str, pubkey: &str) -> bool {
    if pubkey.is_empty() {
        return false;
    }
    p_tag_pubkeys(tags_json).iter().any(|p| p == pubkey)
}

/// A chat event is backend↔party traffic when its author OR any directed `p`-tag
/// recipient is a backend — either this daemon's own management key
/// (`backend_pubkey`, reliable on a cold cache) or a pubkey whose cached kind:0
/// declares `is_backend` (covers remote backends). Such traffic is excluded from
/// ambient `<chatter>`, symmetric with the roster's backend exclusion in
/// `people`/`assemble::member_rows`. Applied identically on both the live
/// (`message_rows`) and captured (`capture::capture_messages`) paths so the two
/// stay in `assert_incremental_equals_full` parity.
pub(crate) fn is_backend_traffic(
    store: &Store,
    backend_pubkey: &str,
    author: &str,
    tags_json: &str,
) -> bool {
    if is_backend_pubkey(store, backend_pubkey, author) {
        return true;
    }
    p_tag_pubkeys(tags_json)
        .iter()
        .any(|pk| is_backend_pubkey(store, backend_pubkey, pk))
}

pub(crate) fn is_backend_pubkey(store: &Store, backend_pubkey: &str, pubkey: &str) -> bool {
    (!backend_pubkey.is_empty() && pubkey == backend_pubkey) || is_backend(store, pubkey)
}

fn is_backend(store: &Store, pubkey: &str) -> bool {
    store
        .get_profile(pubkey)
        .ok()
        .flatten()
        .map(|p| p.is_backend)
        .unwrap_or(false)
}
