use crate::session::DerivedStatus;
use crate::state::Store;
use std::collections::BTreeSet;

mod helpers;
use helpers::*;

const ACTIVITY_LIMIT: u32 = 5;
/// Max parent links to walk when building a channel breadcrumb (cycle guard).
const MAX_BREADCRUMB_DEPTH: usize = 16;

/// One agent whose live status changed (updated_at > since) within a channel.
/// Replaces the old cross-module `StatusDeltaItem`: the awareness layer only
/// needs the channel, the agent slug, and the derived view.
pub(super) struct StatusChange {
    pub(super) channel_h: String,
    pub(super) slug: String,
    /// The agent's host (from its kind:0 profile), so a remote peer renders as
    /// `@slug@host`. Empty when unknown → treated as local.
    pub(super) host: String,
    pub(super) derived: DerivedStatus,
}

// ── channel topology (reconstructed from relay_channels primitives) ───────────

/// Breadcrumb from the root project down to `project`, as `(channel_h, name)`
/// pairs (root first). Empty when the channel is not yet materialized — the
/// caller treats that as "no fabric context to show".
pub(super) fn channel_breadcrumb(store: &Store, project: &str) -> Vec<(String, String)> {
    if store.get_channel(project).ok().flatten().is_none() {
        return Vec::new();
    }
    let mut chain = Vec::new();
    let mut cur = project.to_string();
    for _ in 0..MAX_BREADCRUMB_DEPTH {
        let name = store
            .get_channel(&cur)
            .ok()
            .flatten()
            .map(|c| c.name)
            .unwrap_or_default();
        chain.push((cur.clone(), name));
        match store.channel_parent(&cur).ok().flatten() {
            Some(parent) if !parent.is_empty() => cur = parent,
            _ => break,
        }
    }
    chain.reverse();
    chain
}

/// Direct child channels of `project` as `(channel_h, name, member_count)`.
pub(super) fn subchannels_of(store: &Store, project: &str) -> Vec<(String, String, usize)> {
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == project)
        .map(|c| {
            let count = store.count_channel_members(&c.channel_h).unwrap_or(0) as usize;
            (c.channel_h, c.name, count)
        })
        .collect()
}

// ── line builders ─────────────────────────────────────────────────────────────

/// `Project:` line: the ROOT channel's human name + its description. The raw
/// opaque `channel_h` never appears here.
pub(super) fn project_line(store: &Store, breadcrumb: &[(String, String)], now: u64) -> String {
    let root = &breadcrumb[0].0;
    let name = channel_name_bare(store, root, now);
    match channel_about(store, root).filter(|s| !s.is_empty()) {
        Some(about) => format!("{name} -- {about}"),
        None => name,
    }
}

/// `Channel:` line: the current channel as a project-RELATIVE slash path (root
/// prefix dropped) plus its description. A direct child of the project shows just
/// its name; a deeper channel shows `parent/child`. When the current channel IS
/// the project root, shows the root name.
pub(super) fn channel_path_line(
    store: &Store,
    breadcrumb: &[(String, String)],
    now: u64,
) -> String {
    let names: Vec<String> = breadcrumb
        .iter()
        .map(|(id, _)| channel_name_bare(store, id, now))
        .collect();
    let path = if names.len() <= 1 {
        names.last().cloned().unwrap_or_default()
    } else {
        names[1..].join("/")
    };
    let current = &breadcrumb[breadcrumb.len() - 1].0;
    match channel_about(store, current).filter(|s| !s.is_empty()) {
        Some(about) => format!("{path} -- {about}"),
        None => path,
    }
}

pub(super) fn channel_summary_line(store: &Store, id: &str, now: u64) -> String {
    let count = channel_member_count(store, id, now);
    let (handle, desc) = channel_label(store, id, now);
    match desc {
        Some(d) => format!("{handle} -- {d} [{}]", member_count_label(count)),
        None => format!("{handle} [{}]", member_count_label(count)),
    }
}

/// `#<name>` reference for a channel — name-based, never the raw opaque id.
pub(super) fn channel_ref(store: &Store, id: &str, now: u64) -> String {
    channel_label(store, id, now).0
}

pub(super) fn member_lines(
    store: &Store,
    project: &str,
    now: u64,
    self_slug: &str,
    self_pubkey: &str,
    local_host: &str,
) -> Vec<String> {
    let status_map = super::super::channel::channel_status_map(store, project, now);
    let mut members: Vec<(String, String)> = store
        .list_channel_members(project)
        .unwrap_or_default()
        .into_iter()
        .map(|m| (m.pubkey, m.role))
        .collect();
    let mut seen = members
        .iter()
        .map(|(pubkey, _)| pubkey.clone())
        .collect::<BTreeSet<_>>();
    if !self_pubkey.is_empty() && !members.iter().any(|(pk, _)| pk == self_pubkey) {
        members.push((self_pubkey.to_string(), "member".to_string()));
        seen.insert(self_pubkey.to_string());
    }
    for pubkey in status_map.keys().collect::<BTreeSet<_>>() {
        if !seen.contains(pubkey.as_str()) {
            members.push((pubkey.to_string(), "member".to_string()));
        }
    }
    members
        .into_iter()
        .filter(|(pubkey, _)| !is_backend(store, pubkey))
        .map(|(pubkey, role)| {
            let is_self = !self_pubkey.is_empty() && pubkey == self_pubkey;
            let (slug, host) = if is_self {
                (self_slug.to_string(), local_host.to_string())
            } else {
                (
                    slug_for_pubkey(store, &pubkey),
                    host_for_pubkey(store, &pubkey),
                )
            };
            let you = if is_self { " (you)" } else { "" };
            let status = status_map
                .get(&pubkey)
                .map(|s| super::super::render::status_plain(&s.title, &s.activity, s.busy))
                .unwrap_or_else(|| offline_label(&role));
            let reference = crate::idref::agent_ref_from(&slug, &host, local_host);
            format!("@{reference}{you} - {status}")
        })
        .collect()
}

pub(super) fn changed_status_items(
    store: &Store,
    since: u64,
    project: &str,
    now: u64,
    exclude_pubkey: Option<&str>,
) -> Vec<StatusChange> {
    let subs = subchannels_of(store, project);
    let mut channels = vec![project.to_string()];
    channels.extend(subs.iter().map(|(id, _, _)| id.clone()));
    let mut out = Vec::new();
    for ch in &channels {
        for st in store.live_status_for_channel(ch, now).unwrap_or_default() {
            // The viewer's own status (same signing pubkey) is never echoed back.
            if exclude_pubkey == Some(st.pubkey.as_str()) {
                continue;
            }
            // Only rows that actually changed since the cursor are "new".
            if st.updated_at <= since {
                continue;
            }
            out.push(StatusChange {
                channel_h: ch.clone(),
                slug: peer_slug(store, &st),
                host: host_for_pubkey(store, &st.pubkey),
                derived: super::super::channel::derive_from_status(&st, now),
            });
        }
    }
    out
}

pub(super) fn changed_member_lines(
    project: &str,
    items: &[StatusChange],
    local_host: &str,
) -> Vec<String> {
    items
        .iter()
        .filter(|item| item.channel_h == project)
        .filter_map(|item| useful_work_text(item).map(|status| (item, status)))
        .map(|(item, status)| {
            let reference = crate::idref::agent_ref_from(&item.slug, &item.host, local_host);
            format!("@{reference} - {status}")
        })
        .collect()
}

pub(super) fn changed_subchannel_lines(
    store: &Store,
    since: u64,
    project: &str,
    now: u64,
    subs: &[(String, String, usize)],
    items: &[StatusChange],
) -> Vec<String> {
    let active: BTreeSet<String> = active_channels_since(store, since, &[project.to_string()]);
    let changed: BTreeSet<String> = items
        .iter()
        .map(|item| item.channel_h.clone())
        .filter(|id| id != project)
        .collect();
    subs.iter()
        .map(|(id, _, _)| id)
        .filter(|id| active.contains(*id) || changed.contains(*id))
        .map(|id| channel_summary_line(store, id, now))
        .collect()
}

/// The "Other active channels" list: the TOP-LEVEL channels of the viewer's own
/// project (the direct children of the project root) that saw activity since
/// `cutoff`, MINUS the top-level branch the viewer is currently inside. The store
/// is machine-global (one daemon owns every project's data), so this is the read
/// layer that scopes awareness back down to a single project:
///
/// - channels belonging to a DIFFERENT project (their root ≠ ours) are skipped;
/// - channels whose ancestry can't be traced to a materialized root are DROPPED
///   with a loud warning — they'd otherwise leak across project boundaries.
///
/// Deeper-nested rooms of our own project are not listed here (they surface under
/// `Subchannels:` when the viewer is inside their branch).
pub(super) fn other_active_channel_lines(
    store: &Store,
    project: &str,
    cutoff: u64,
    now: u64,
) -> Vec<String> {
    // The viewer is anchored at `project`; treat it as its own root if its row
    // isn't materialized (an un-cached root project), so we never warn about it.
    let root = resolve_root(store, project).unwrap_or_else(|| project.to_string());
    let current_branch = top_level_branch(store, project, &root);
    let mut channels: Vec<String> = store
        .active_channels_since(cutoff)
        .unwrap_or_default()
        .into_iter()
        .filter(|id| id != project)
        .filter(|id| match resolve_root(store, id) {
            None => {
                tracing::warn!(
                    channel = %id,
                    project_root = %root,
                    "[tenex-edge] awareness: DROPPING active channel with unresolvable \
                     project root (unmaterialized ancestry) — refusing to leak it across \
                     project boundaries into \"Other active channels\""
                );
                false
            }
            // A different project's channel — silently out of scope.
            Some(r) if r != root => false,
            // Same project: keep only top-level branches (direct children of the
            // root), excluding the branch the viewer is already in.
            Some(_) => {
                store.channel_parent(id).ok().flatten().as_deref() == Some(root.as_str())
                    && Some(id.as_str()) != current_branch.as_deref()
            }
        })
        .collect();
    channels.sort();
    channels
        .into_iter()
        .take(5)
        .map(|id| channel_summary_line(store, &id, now))
        .collect()
}

pub(super) fn current_activity_lines(
    store: &Store,
    project: &str,
    since: u64,
    now: u64,
    exclude_pubkey: Option<&str>,
    local_host: &str,
) -> Vec<String> {
    store
        .chat_for_channel(project, since, ACTIVITY_LIMIT)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| exclude_pubkey != Some(row.pubkey.as_str()))
        .map(|row| activity_line(store, row, now, local_host))
        .collect()
}

fn member_count_label(count: usize) -> String {
    match count {
        1 => "1 member".to_string(),
        n => format!("{n} members"),
    }
}

fn offline_label(role: &str) -> String {
    if role == "admin" {
        "Human".to_string()
    } else {
        "offline".to_string()
    }
}
