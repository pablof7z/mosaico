//! The PURE fabric-view derivation: a faithful port of `build_view`/`people`/
//! `messages` that reads ONLY the frozen [`ViewInputs`] plus the two explicit
//! inputs — the seen `cursor` and the wall-clock `now`. No store, no ambient
//! time. This is the function the Trellis reconciler runs inside a derived node,
//! so `why_changed`/`why_output_frame` attribute the snapshot to exactly these
//! inputs and it replays byte-for-byte from the same inputs.
//!
//! The full-vs-delta SHAPE decision keys on `cursor` (`cursor == 0` → full
//! `<members>`/`<subchannels>`; `cursor > 0` → delta `<recent-presence>` only);
//! every time-relative string keys on `now`. Equivalence with the legacy
//! `build_view` is asserted in the reconciler's regression test.

use std::collections::HashSet;

use super::capture::{EvCap, MsgBundle, StatusCap, ViewInputs};
use super::model::*;
use crate::util::relative_time;

mod members;

/// Chat lookback window used on the full (first-turn) render.
const WINDOW_SECS: u64 = 4 * 60 * 60;
const MAX_CLUSTER_GAP_SECS: u64 = 20 * 60;
const MAX_CLUSTER_ROWS: usize = 30;

/// Derive the fabric view from the canonical inputs + explicit `cursor`/`now`.
pub(crate) fn assemble_view(inputs: &ViewInputs, cursor: u64, now: u64) -> FabricView {
    let meta = &inputs.meta;
    let mut view = FabricView {
        self_row: meta.self_row.as_ref().map(|s| SelfRow {
            agent: s.agent.clone(),
            agent_slug: s.agent_slug.clone(),
            host: s.host.clone(),
        }),
        workspace: WorkspaceRow {
            name: meta.workspace.name.clone(),
            about: meta.workspace.about.clone(),
        },
        agents: agent_rows(inputs, cursor, now),
        channels: Vec::new(),
        unjoined: unjoined_rows(inputs, now),
        important: Vec::new(),
        warnings: meta
            .warnings
            .iter()
            .cloned()
            .map(|text| WarningRow { text })
            .collect(),
        incremental: cursor != 0,
    };

    let full = cursor == 0;
    for chan in &meta.channels {
        let (messages, omitted) = inputs
            .messages
            .channels
            .get(&chan.h)
            .map(|bundle| message_rows(bundle, cursor, now))
            .unwrap_or_default();
        let presence = presence_rows(inputs, &chan.h, cursor, now);
        if !full && messages.is_empty() && presence.is_empty() {
            continue;
        }
        for msg in &messages {
            if msg.mention {
                view.important.push(ImportantRow {
                    channel: msg.channel.clone(),
                    message_id: msg.id.clone(),
                });
            }
        }
        view.channels.push(ChannelBlock {
            name: chan.name.clone(),
            about: chan.about.clone(),
            members: if full {
                members::member_rows(inputs, &chan.h, now)
            } else {
                Vec::new()
            },
            presence,
            subchannels: if full {
                chan.subchannels
                    .iter()
                    .map(|s| ChannelSummaryRow {
                        name: s.name.clone(),
                        about: s.about.clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            messages,
            omitted,
        });
    }
    view
}

fn agent_rows(inputs: &ViewInputs, cursor: u64, now: u64) -> Vec<AgentRow> {
    inputs
        .meta
        .agents
        .iter()
        .filter(|a| cursor == 0 || (a.created_at > cursor && a.created_at <= now))
        .map(|a| AgentRow {
            reference: a.reference.clone(),
            about: a.about.clone(),
        })
        .collect()
}

fn unjoined_rows(inputs: &ViewInputs, now: u64) -> Vec<UnjoinedChannelRow> {
    inputs
        .meta
        .unjoined
        .iter()
        .map(|u| UnjoinedChannelRow {
            name: u.name.clone(),
            about: u.about.clone(),
            last_active: relative_time(u.updated_at, now),
        })
        .collect()
}

fn presence_rows(inputs: &ViewInputs, channel: &str, cursor: u64, now: u64) -> Vec<PresenceRow> {
    if cursor == 0 {
        return Vec::new();
    }
    let self_pubkey = &inputs.meta.self_pubkey;
    inputs
        .presence
        .statuses
        .get(channel)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|s| s.expiration >= now)
        .filter(|s| s.updated_at > cursor)
        .filter(|s| &s.pubkey != self_pubkey)
        .map(|s| PresenceRow {
            reference: presence_reference(inputs, s),
            status: status_text(s),
            seen: relative_time(s.last_seen, now),
        })
        .collect()
}

fn presence_reference(inputs: &ViewInputs, status: &StatusCap) -> String {
    if !status.session_id.is_empty() {
        let profile_agent_slug = inputs
            .members
            .agent_slugs
            .get(&status.pubkey)
            .map(String::as_str)
            .unwrap_or("");
        return crate::fabric_context::refs::session_ref(
            &status.session_id,
            &status.slug,
            profile_agent_slug,
        );
    }
    inputs
        .members
        .refs
        .get(&status.pubkey)
        .cloned()
        .unwrap_or_default()
}

fn status_text(status: &StatusCap) -> String {
    if status.busy {
        return non_empty(&status.activity)
            .or_else(|| non_empty(&status.title))
            .unwrap_or_else(|| "working".to_string());
    }
    non_empty(&status.title).unwrap_or_else(|| "idle".to_string())
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn message_rows(bundle: &MsgBundle, cursor: u64, now: u64) -> (Vec<MessageRow>, usize) {
    let since = if cursor == 0 {
        now.saturating_sub(WINDOW_SECS)
    } else {
        cursor
    };
    let mut events: Vec<EvCap> = bundle
        .events
        .iter()
        .filter(|e| e.created_at > since)
        .cloned()
        .collect();
    let omitted = if cursor == 0 {
        let total = events.len();
        events = recent_cluster(events);
        total.saturating_sub(events.len())
    } else {
        0
    };
    let mut seen: HashSet<String> = events.iter().map(|e| e.id.clone()).collect();
    for f in &bundle.forced {
        if seen.insert(f.id.clone()) {
            events.push(f.clone());
        }
    }
    events.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let forced_mentions: HashSet<&str> = bundle
        .forced
        .iter()
        .filter(|f| f.forced_mention)
        .map(|f| f.id.as_str())
        .collect();
    let rows = events
        .into_iter()
        .map(|e| MessageRow {
            mention: e.mentions_self || forced_mentions.contains(e.id.as_str()),
            age: relative_time(e.created_at, now),
            id: e.id,
            channel: e.channel_display,
            from: e.from_ref,
            recipients: e.recipient_refs,
            body: e.body,
            truncated: e.truncated,
        })
        .collect();
    (rows, omitted)
}

fn recent_cluster(mut events: Vec<EvCap>) -> Vec<EvCap> {
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
