//! Leaf store readers for [`super::capture_inputs`]. Each mirrors the exact
//! store read `build_view`/`people`/`messages` performs, resolving now/cursor-
//! independent data (names, refs, membership, raw rows) into owned capture
//! structs. No `now`/`cursor` filtering happens here.

use std::collections::{BTreeMap, BTreeSet};

use super::{AgentCap, EvCap, MsgBundle, SelfCap, SummaryCap, UnjoinedCap};
use crate::fabric_context::messages::mentions_pubkey;
use crate::fabric_context::refs::{display_name, pubkey_ref};
use crate::fabric_context::{FabricContextInput, FabricMessageSeed};
use crate::state::{Session, Store};
use crate::util::{truncate_words, CHAT_RENDER_WORD_LIMIT};

/// Widest chat capture cap; assemble re-applies the real per-window limit.
const CHAT_CAPTURE_CAP: u32 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChannelReadiness {
    Ready,
    Archived,
    Missing,
}

pub(super) fn self_cap(s: &Session, input: &FabricContextInput<'_>) -> SelfCap {
    SelfCap {
        agent: input.self_slug.to_string(),
        backend: input.local_host.to_string(),
        session_id: s.session_id.clone(),
    }
}

/// The ordered, deduped, archived-pruned channel set — identical to the head of
/// `build_view` (joined channels ∪ forced-message channels, minus archived).
pub(super) fn selected_channels(store: &Store, input: &FabricContextInput<'_>) -> Vec<String> {
    let mut channels = channels_for(store, input.session, input.scope);
    let forced_by_channel = group_forced(input.forced_messages, input.scope);
    for ch in forced_by_channel.keys() {
        if !channels.iter().any(|c| c == ch) {
            channels.push(ch.clone());
        }
    }
    channels.sort();
    channels.dedup();
    channels.retain(|channel| matches!(channel_readiness(store, channel), ChannelReadiness::Ready));
    channels
}

pub(super) fn missing_channels(store: &Store, input: &FabricContextInput<'_>) -> Vec<String> {
    let mut channels = channels_for(store, input.session, input.scope);
    let forced_by_channel = group_forced(input.forced_messages, input.scope);
    for ch in forced_by_channel.keys() {
        if !channels.iter().any(|c| c == ch) {
            channels.push(ch.clone());
        }
    }
    channels.sort();
    channels.dedup();
    channels
        .into_iter()
        .filter(|channel| matches!(channel_readiness(store, channel), ChannelReadiness::Missing))
        .collect()
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

pub(super) fn group_forced(
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

pub(super) fn subchannel_caps(store: &Store, channel: &str) -> Vec<SummaryCap> {
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == channel && !c.is_archived())
        .map(|c| SummaryCap {
            name: c.human_name().unwrap_or(&c.channel_h).to_string(),
            about: c.about,
        })
        .filter(|c| !c.name.is_empty())
        .collect()
}

pub(super) fn unjoined_caps(
    store: &Store,
    root: &str,
    joined_channels: &[String],
) -> Vec<UnjoinedCap> {
    let joined = joined_channels.iter().cloned().collect::<BTreeSet<_>>();
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == root && !joined.contains(&c.channel_h) && !c.is_archived())
        .filter_map(|c| {
            let name = c.human_name().unwrap_or(&c.channel_h).to_string();
            (!name.is_empty()).then_some(UnjoinedCap {
                name,
                about: c.about,
                updated_at: c.updated_at,
            })
        })
        .take(12)
        .collect()
}

pub(super) fn agent_caps(
    store: &Store,
    root: &str,
    input: &FabricContextInput<'_>,
) -> Vec<AgentCap> {
    store
        .list_agent_roster_for_channel(root)
        .unwrap_or_default()
        .into_iter()
        .map(|row| AgentCap {
            reference: if row.host.is_empty() || row.host == input.local_host {
                row.slug
            } else {
                format!("{}@{}", row.slug, row.host)
            },
            about: row.use_criteria,
            created_at: row.updated_at,
        })
        .collect()
}

pub(super) fn capture_messages(
    store: &Store,
    input: &FabricContextInput<'_>,
    channel: &str,
    forced: &[FabricMessageSeed],
) -> MsgBundle {
    if input.session.is_none() {
        return MsgBundle::default();
    }
    let events = store
        .chat_for_channel(channel, 0, CHAT_CAPTURE_CAP)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.kind == crate::fabric::nip29::wire::KIND_CHAT as u32)
        .filter(|e| e.pubkey != input.self_pubkey)
        .map(|ev| {
            let (body, truncated) = truncate_words(&ev.content, CHAT_RENDER_WORD_LIMIT);
            EvCap {
                id: ev.id.clone(),
                channel_display: display_name(store, &ev.channel_h),
                from_ref: pubkey_ref(store, &ev.pubkey, input.local_host),
                created_at: ev.created_at,
                body,
                truncated,
                mentions_self: mentions_pubkey(&ev.tags_json, input.self_pubkey),
                forced_mention: false,
            }
        })
        .collect();
    let forced = forced
        .iter()
        .map(|row| {
            let (body, truncated) = truncate_words(&row.body, CHAT_RENDER_WORD_LIMIT);
            EvCap {
                id: row.id.clone(),
                channel_display: display_name(store, channel),
                from_ref: pubkey_ref(store, &row.from_pubkey, input.local_host),
                created_at: row.created_at,
                body,
                truncated,
                mentions_self: false,
                forced_mention: row.mention,
            }
        })
        .collect();
    MsgBundle { events, forced }
}

pub(super) fn resolve_pubkey(
    store: &Store,
    pubkey: &str,
    local_host: &str,
    refs: &mut BTreeMap<String, String>,
    backend: &mut BTreeSet<String>,
) {
    if refs.contains_key(pubkey) {
        return;
    }
    refs.insert(pubkey.to_string(), pubkey_ref(store, pubkey, local_host));
    if is_backend(store, pubkey) {
        backend.insert(pubkey.to_string());
    }
}

pub(super) fn project_root(store: &Store, channel: &str) -> String {
    store
        .channel_project_root(channel)
        .ok()
        .flatten()
        .unwrap_or_else(|| channel.to_string())
}

fn is_backend(store: &Store, pubkey: &str) -> bool {
    store
        .get_profile(pubkey)
        .ok()
        .flatten()
        .map(|p| p.is_backend)
        .unwrap_or(false)
}

pub(super) fn channel_summary(store: &Store, channel: &str) -> SummaryCap {
    let ch = store
        .get_channel(channel)
        .ok()
        .flatten()
        .expect("renderable channels are filtered through get_channel first");
    SummaryCap {
        name: ch
            .human_name()
            .map(str::to_string)
            .unwrap_or_else(|| display_name(store, channel)),
        about: ch.about,
    }
}

pub(super) fn project_summary(store: &Store, channel: &str) -> SummaryCap {
    let ch = store.get_channel(channel).ok().flatten();
    SummaryCap {
        name: ch
            .as_ref()
            .and_then(|c| c.human_name())
            .map(str::to_string)
            .unwrap_or_else(|| display_name(store, channel)),
        about: ch.map(|c| c.about).unwrap_or_default(),
    }
}

fn channel_readiness(store: &Store, channel: &str) -> ChannelReadiness {
    match store.get_channel(channel) {
        Ok(Some(ch)) if !ch.is_archived() => ChannelReadiness::Ready,
        Ok(Some(_)) => ChannelReadiness::Archived,
        _ => ChannelReadiness::Missing,
    }
}
