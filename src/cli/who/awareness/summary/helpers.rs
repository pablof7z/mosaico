use super::{StatusChange, MAX_BREADCRUMB_DEPTH};
use crate::state::{RelayEvent, Store};
use crate::util::{pubkey_short, relative_time};
use std::collections::BTreeSet;

/// Channels (other than `exclude`) whose status changed since `cutoff`.
pub(super) fn active_channels_since(
    store: &Store,
    cutoff: u64,
    exclude: &[String],
) -> BTreeSet<String> {
    let excl: BTreeSet<&str> = exclude.iter().map(String::as_str).collect();
    store
        .active_channels_since(cutoff)
        .unwrap_or_default()
        .into_iter()
        .filter(|id| !excl.contains(id.as_str()))
        .collect()
}

pub(super) fn channel_about(store: &Store, id: &str) -> Option<String> {
    store
        .get_channel(id)
        .ok()
        .flatten()
        .map(|c| c.about.trim().to_string())
}

pub(super) fn channel_label(store: &Store, id: &str, now: u64) -> (String, Option<String>) {
    if let Some(channel) = store.get_channel(id).ok().flatten() {
        if let Some(name) = channel.human_name() {
            let about = channel.about.trim();
            return (
                format!("#{name}"),
                (!about.is_empty()).then(|| about.to_string()),
            );
        }
    }
    (unnamed_channel_label(store, id, now), None)
}

pub(super) fn channel_name_bare(store: &Store, id: &str, now: u64) -> String {
    if let Some(channel) = store.get_channel(id).ok().flatten() {
        if let Some(name) = channel.human_name() {
            return name.to_string();
        }
    }
    unnamed_channel_label(store, id, now)
}

fn unnamed_channel_label(store: &Store, id: &str, now: u64) -> String {
    latest_channel_work_title(store, id, now)
        .or_else(|| channel_about(store, id).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "(unnamed channel)".to_string())
}

fn latest_channel_work_title(store: &Store, id: &str, now: u64) -> Option<String> {
    store
        .live_status_for_channel(id, now)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.title.trim().to_string())
        .find(|t| !t.is_empty() && t != id)
}

pub(super) fn channel_member_count(store: &Store, id: &str, now: u64) -> usize {
    let n = store
        .list_channel_members(id)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| !is_backend(store, &m.pubkey))
        .count();
    if n > 0 {
        return n;
    }
    super::super::super::channel::channel_status_map(store, id, now)
        .values()
        .filter(|status| status.liveness.is_live())
        .count()
}

pub(super) fn resolve_root(store: &Store, channel: &str) -> Option<String> {
    let mut cur = channel.to_string();
    for _ in 0..MAX_BREADCRUMB_DEPTH {
        match store.channel_parent(&cur).ok().flatten() {
            Some(p) if p.is_empty() => return Some(cur),
            Some(p) => cur = p,
            None => return None,
        }
    }
    None
}

pub(super) fn top_level_branch(store: &Store, channel: &str, root: &str) -> Option<String> {
    if channel == root {
        return None;
    }
    let mut cur = channel.to_string();
    for _ in 0..MAX_BREADCRUMB_DEPTH {
        match store.channel_parent(&cur).ok().flatten() {
            Some(p) if p == root => return Some(cur),
            Some(p) if !p.is_empty() => cur = p,
            _ => return None,
        }
    }
    None
}

pub(super) fn useful_work_text(item: &StatusChange) -> Option<String> {
    if item.derived.title.is_empty() && item.derived.activity.is_empty() {
        return None;
    }
    Some(super::super::super::render::status_plain(
        &item.derived.title,
        &item.derived.activity,
        item.derived.busy,
    ))
}

pub(super) fn activity_line(store: &Store, row: RelayEvent, now: u64, local_host: &str) -> String {
    let slug = slug_for_pubkey(store, &row.pubkey);
    let host = host_for_pubkey(store, &row.pubkey);
    let from = crate::idref::agent_ref_from(&slug, &host, local_host);
    let content = crate::profile::rewrite_body_mentions(store, &row.content);
    format!(
        "[@{from}, {}] {}",
        relative_time(row.created_at, now),
        content
    )
}

pub(super) fn host_for_pubkey(store: &Store, pubkey: &str) -> String {
    store
        .get_profile(pubkey)
        .ok()
        .flatten()
        .map(|p| p.host)
        .unwrap_or_default()
}

pub(super) fn is_backend(store: &Store, pubkey: &str) -> bool {
    store
        .get_profile(pubkey)
        .ok()
        .flatten()
        .map(|p| p.is_backend)
        .unwrap_or(false)
}

pub(super) fn peer_slug(store: &Store, st: &crate::state::Status) -> String {
    if !st.slug.is_empty() {
        return st.slug.clone();
    }
    slug_for_pubkey(store, &st.pubkey)
}

pub(super) fn slug_for_pubkey(store: &Store, pubkey: &str) -> String {
    store
        .resolve_slug_for_pubkey(pubkey)
        .ok()
        .flatten()
        .unwrap_or_else(|| pubkey_short(pubkey))
}
