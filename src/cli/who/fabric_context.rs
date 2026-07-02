use crate::state::{InboxRow, RelayEvent, Session, Status, Store};
use crate::util::{relative_time, truncate_words, CHAT_RENDER_WORD_LIMIT};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

mod model;
mod render;
#[cfg(test)]
mod tests;

use model::*;
use render::render_view;

const WINDOW_SECS: u64 = 4 * 60 * 60;
const CHAT_CAP: u32 = 10_000;
const MAX_CLUSTER_GAP_SECS: u64 = 20 * 60;
const MAX_CLUSTER_ROWS: usize = 30;

pub(crate) struct FabricContextInput<'a> {
    pub(crate) session: Option<&'a Session>,
    pub(crate) scope: &'a str,
    pub(crate) cursor: u64,
    pub(crate) now: u64,
    pub(crate) self_slug: &'a str,
    pub(crate) self_pubkey: &'a str,
    pub(crate) local_host: &'a str,
    pub(crate) edge_home: Option<&'a Path>,
    pub(crate) forced_messages: &'a [FabricMessageSeed],
    pub(crate) warnings: &'a [String],
    pub(crate) force: bool,
}

#[derive(Clone)]
pub(crate) struct FabricMessageSeed {
    pub(crate) id: String,
    pub(crate) channel: String,
    pub(crate) from_pubkey: String,
    pub(crate) body: String,
    pub(crate) created_at: u64,
    pub(crate) mention: bool,
}

pub(crate) fn render_fabric_context(
    store: &Store,
    input: FabricContextInput<'_>,
) -> Option<String> {
    let force = input.force;
    let view = build_view(store, input);
    if !force
        && view.channels.is_empty()
        && view.agents.is_empty()
        && view.important.is_empty()
        && view.warnings.is_empty()
    {
        return None;
    }
    Some(render_view(&view))
}

pub(crate) fn inbox_seed(row: &InboxRow) -> FabricMessageSeed {
    FabricMessageSeed {
        id: row.event_id.clone(),
        channel: row.channel_h.clone(),
        from_pubkey: row.from_pubkey.clone(),
        body: row.body.clone(),
        created_at: row.created_at,
        mention: true,
    }
}

fn build_view(store: &Store, input: FabricContextInput<'_>) -> FabricView {
    let root = project_root(store, input.scope);
    let project = channel_summary(store, &root, input.now);
    let mut channels = channels_for(store, input.session, input.scope);
    let forced_by_channel = group_forced(input.forced_messages, input.scope);
    for ch in forced_by_channel.keys() {
        if !channels.iter().any(|c| c == ch) {
            channels.push(ch.clone());
        }
    }
    channels.sort();
    channels.dedup();

    let mut view = FabricView {
        self_row: input.session.map(|s| SelfRow {
            agent: input.self_slug.to_string(),
            backend: input.local_host.to_string(),
            session_id: s.session_id.clone(),
        }),
        project,
        agents: agents(input.edge_home, input.cursor, input.now),
        channels: Vec::new(),
        inactive: inactive_channels(store, &root, &channels, input.now),
        important: Vec::new(),
        warnings: input
            .warnings
            .iter()
            .cloned()
            .map(|text| WarningRow { text })
            .collect(),
    };

    for channel in channels {
        let forced = forced_by_channel.get(&channel).cloned().unwrap_or_default();
        let messages = if input.session.is_some() {
            message_rows(store, &input, &channel, &forced)
        } else {
            (Vec::new(), 0)
        };
        let presence = presence_rows(store, &channel, &input);
        let full = input.cursor == 0;
        if !full && messages.0.is_empty() && presence.is_empty() {
            continue;
        }
        for msg in &messages.0 {
            if msg.mention {
                view.important.push(ImportantRow {
                    channel: msg.channel.clone(),
                    message_id: msg.id.clone(),
                });
            }
        }
        let summary = channel_summary(store, &channel, input.now);
        view.channels.push(ChannelBlock {
            id: channel.clone(),
            name: summary.name,
            about: summary.about,
            active: channel == input.scope,
            members: if full {
                member_rows(store, &channel, &input)
            } else {
                Vec::new()
            },
            presence,
            subchannels: if full {
                subchannel_rows(store, &channel)
            } else {
                Vec::new()
            },
            messages: messages.0,
            omitted: messages.1,
        });
    }
    view
}

fn channels_for(store: &Store, session: Option<&Session>, scope: &str) -> Vec<String> {
    let Some(rec) = session else {
        return vec![scope.to_string()];
    };
    let mut channels = store
        .list_session_joined_channels(&rec.session_id)
        .unwrap_or_else(|_| vec![(rec.channel_h.clone(), rec.created_at)])
        .into_iter()
        .map(|(h, _)| h)
        .collect::<Vec<_>>();
    if !channels.iter().any(|h| h == scope) {
        channels.push(scope.to_string());
    }
    channels
}

fn group_forced(
    rows: &[FabricMessageSeed],
    fallback_scope: &str,
) -> BTreeMap<String, Vec<FabricMessageSeed>> {
    let mut out: BTreeMap<String, Vec<FabricMessageSeed>> = BTreeMap::new();
    for row in rows {
        let channel = if row.channel.is_empty() {
            fallback_scope
        } else {
            &row.channel
        };
        out.entry(channel.to_string())
            .or_default()
            .push(row.clone());
    }
    out
}

fn message_rows(
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
        .collect::<Vec<_>>();
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
                tags_json: "[]".to_string(),
            });
        }
    }
    events.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let rows = events
        .into_iter()
        .map(|ev| {
            let mention = forced.iter().any(|f| f.id == ev.id && f.mention)
                || mentions_pubkey(&ev, input.self_pubkey);
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
    let (body, truncated) = truncate_words(&ev.content, CHAT_RENDER_WORD_LIMIT);
    MessageRow {
        id: ev.id.clone(),
        channel: display_name(store, &ev.channel_h, now),
        from: pubkey_ref(store, &ev.pubkey, local_host),
        age: relative_time(ev.created_at, now),
        body,
        mention,
        truncated,
    }
}

fn mentions_pubkey(ev: &RelayEvent, pubkey: &str) -> bool {
    if pubkey.is_empty() {
        return false;
    }
    let Ok(tags) = serde_json::from_str::<Vec<Vec<String>>>(&ev.tags_json) else {
        return false;
    };
    tags.iter()
        .any(|tag| tag.first().is_some_and(|t| t == "p") && tag.get(1).is_some_and(|p| p == pubkey))
}

fn member_rows(store: &Store, channel: &str, input: &FabricContextInput<'_>) -> Vec<MemberRow> {
    let statuses = status_map(store, channel, input.now);
    let mut pubkeys = store
        .list_channel_members(channel)
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.pubkey)
        .collect::<BTreeSet<_>>();
    pubkeys.extend(statuses.keys().cloned());
    if !input.self_pubkey.is_empty() {
        pubkeys.insert(input.self_pubkey.to_string());
    }
    pubkeys
        .into_iter()
        .filter(|pk| !is_backend(store, pk))
        .map(|pk| {
            let status = statuses
                .get(&pk)
                .map(status_text)
                .unwrap_or_else(|| "offline".to_string());
            let seen = statuses
                .get(&pk)
                .map(|s| relative_time(s.last_seen, input.now))
                .unwrap_or_else(|| "unknown".to_string());
            MemberRow {
                reference: if pk == input.self_pubkey {
                    crate::idref::agent_ref_from(
                        input.self_slug,
                        input.local_host,
                        input.local_host,
                    )
                } else {
                    pubkey_ref(store, &pk, input.local_host)
                },
                status,
                seen,
            }
        })
        .collect()
}

fn presence_rows(store: &Store, channel: &str, input: &FabricContextInput<'_>) -> Vec<PresenceRow> {
    if input.cursor == 0 {
        return Vec::new();
    }
    store
        .live_status_for_channel(channel, input.now)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.updated_at > input.cursor)
        .filter(|s| s.pubkey != input.self_pubkey)
        .map(|s| PresenceRow {
            reference: pubkey_ref(store, &s.pubkey, input.local_host),
            status: status_text(&s),
            seen: relative_time(s.last_seen, input.now),
        })
        .collect()
}

fn status_map(store: &Store, channel: &str, now: u64) -> BTreeMap<String, Status> {
    store
        .live_status_for_channel(channel, now)
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.pubkey.clone(), s))
        .collect()
}

fn status_text(status: &Status) -> String {
    if status.busy {
        return non_empty(&status.activity)
            .or_else(|| non_empty(&status.title))
            .unwrap_or_else(|| "working".to_string());
    }
    non_empty(&status.title).unwrap_or_else(|| "idle".to_string())
}

fn subchannel_rows(store: &Store, channel: &str) -> Vec<ChannelSummaryRow> {
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == channel)
        .map(|c| {
            let name = c.human_name().unwrap_or(&c.channel_h).to_string();
            ChannelSummaryRow {
                name,
                about: c.about,
            }
        })
        .filter(|c| !c.name.is_empty())
        .collect()
}

fn inactive_channels(
    store: &Store,
    root: &str,
    active_channels: &[String],
    now: u64,
) -> Vec<InactiveChannelRow> {
    let active = active_channels.iter().cloned().collect::<BTreeSet<_>>();
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == root && !active.contains(&c.channel_h))
        .filter_map(|c| {
            let name = c.human_name().unwrap_or(&c.channel_h).to_string();
            (!name.is_empty()).then(|| InactiveChannelRow {
                name,
                about: c.about,
                last_active: relative_time(c.updated_at, now),
            })
        })
        .take(12)
        .collect()
}

fn agents(edge_home: Option<&Path>, cursor: u64, now: u64) -> Vec<AgentRow> {
    let Some(edge_home) = edge_home else {
        return Vec::new();
    };
    crate::identity::list_invitable_agents(edge_home)
        .into_iter()
        .filter(|(_, _, created_at)| cursor == 0 || (*created_at > cursor && *created_at <= now))
        .map(|(slug, byline, _)| AgentRow {
            reference: slug,
            about: byline.unwrap_or_default(),
        })
        .collect()
}

fn project_root(store: &Store, channel: &str) -> String {
    store
        .channel_project_root(channel)
        .ok()
        .flatten()
        .unwrap_or_else(|| channel.to_string())
}

fn channel_summary(store: &Store, channel: &str, now: u64) -> ProjectRow {
    let ch = store.get_channel(channel).ok().flatten();
    ProjectRow {
        name: ch
            .as_ref()
            .and_then(|c| c.human_name())
            .map(str::to_string)
            .unwrap_or_else(|| display_name(store, channel, now)),
        about: ch.map(|c| c.about).unwrap_or_default(),
    }
}

fn display_name(store: &Store, channel: &str, _now: u64) -> String {
    store
        .get_channel(channel)
        .ok()
        .flatten()
        .and_then(|c| c.human_name().map(str::to_string))
        .unwrap_or_else(|| channel.to_string())
}

fn pubkey_ref(store: &Store, pubkey: &str, local_host: &str) -> String {
    let profile = store.get_profile(pubkey).ok().flatten();
    let slug = profile
        .as_ref()
        .map(|p| p.slug.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::util::pubkey_short(pubkey));
    let host = profile
        .as_ref()
        .map(|p| p.host.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| local_host.to_string());
    crate::idref::agent_ref_from(&slug, &host, local_host)
}

fn is_backend(store: &Store, pubkey: &str) -> bool {
    store
        .get_profile(pubkey)
        .ok()
        .flatten()
        .map(|p| p.is_backend)
        .unwrap_or(false)
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}
