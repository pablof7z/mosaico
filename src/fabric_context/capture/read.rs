//! Leaf store readers for [`super::capture_inputs`]. They resolve
//! now/cursor-independent data (names, refs, membership, raw rows) into owned
//! capture structs. No `now`/`cursor` filtering happens here.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::{EvCap, MsgBundle, SelfCap};
use crate::fabric_context::messages::{is_backend_traffic, mentions_pubkey, p_tag_pubkeys};
pub(super) use crate::fabric_context::refs::profile_host;
use crate::fabric_context::refs::pubkey_ref;
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

pub(super) fn self_cap(store: &Store, s: &Session, input: &FabricContextInput<'_>) -> SelfCap {
    let resolver = crate::daemon::workspace_path::WorkspacePathResolver::new(store);
    let workspace = s.work_root.clone();
    let branch = resolver
        .path_for_channel(&workspace)
        .ok()
        .flatten()
        .and_then(|path| git_branch(std::path::Path::new(&path)))
        .unwrap_or_default();
    SelfCap {
        name: input.self_slug.to_string(),
        host: input.local_host.to_string(),
        headless: crate::session_host::session_is_headless(store, s),
        title: s.title.clone(),
        workspace,
        branch,
        turn_count: s.turn_count,
    }
}

fn git_branch(workspace: &std::path::Path) -> Option<String> {
    if !workspace.is_dir() {
        return None;
    }
    if let Some(branch) = branch_from_head(workspace) {
        return Some(branch);
    }
    let output = std::process::Command::new("git")
        .args(["-C"])
        .arg(workspace)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8(output.stdout).ok()?;
    let branch = branch.trim();
    (!branch.is_empty() && !branch.chars().any(char::is_control)).then(|| branch.to_string())
}

fn branch_from_head(workspace: &std::path::Path) -> Option<String> {
    let dot_git = workspace.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let pointer = std::fs::read_to_string(dot_git).ok()?;
        let path = pointer.trim().strip_prefix("gitdir:")?.trim();
        let path = std::path::Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace.join(path)
        }
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let branch = head.trim().strip_prefix("ref: refs/heads/")?;
    (!branch.is_empty() && !branch.chars().any(char::is_control)).then(|| branch.to_string())
}

pub(super) fn joined_channels(store: &Store, session: Option<&Session>) -> Vec<String> {
    session_channels(store, session)
        .into_iter()
        .filter(|channel| matches!(channel_readiness(store, channel), ChannelReadiness::Ready))
        .collect()
}

fn session_channels(store: &Store, session: Option<&Session>) -> Vec<String> {
    let Some(rec) = session else {
        return Vec::new();
    };
    let mut channels = store
        .list_session_routes(&rec.pubkey)
        .unwrap_or_default()
        .into_iter()
        .map(|(h, _)| h)
        .filter(|channel| !channel.is_empty())
        .collect::<Vec<_>>();
    channels.sort();
    channels.dedup();
    channels
}

/// The ordered, deduped, archived-pruned channel set: joined channels plus
/// forced-message channels, minus archived channels.
pub(super) fn selected_channels(store: &Store, input: &FabricContextInput<'_>) -> Vec<String> {
    let mut channels = joined_channels(store, input.session);
    if input.session.is_none() && !input.scope.is_empty() {
        channels.push(input.scope.to_string());
    }
    if input.session.is_some()
        && !input.scope.is_empty()
        && !channels.iter().any(|channel| channel == input.scope)
    {
        channels.push(input.scope.to_string());
    }
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
    let mut channels = session_channels(store, input.session);
    if !input.scope.is_empty() && !channels.iter().any(|channel| channel == input.scope) {
        channels.push(input.scope.to_string());
    }
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

pub(super) fn capture_messages(
    store: &Store,
    input: &FabricContextInput<'_>,
    channel: &str,
    forced: &[FabricMessageSeed],
) -> MsgBundle {
    if input.session.is_none() {
        return MsgBundle::default();
    }
    let injected: HashSet<String> = input
        .session
        .and_then(|session| store.injected_for_pubkey(&session.pubkey).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|row| row.event_id)
        .collect();
    let events = store
        .chat_for_channel(channel, 0, CHAT_CAPTURE_CAP)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.kind == crate::fabric::nip29::wire::KIND_CHAT as u32)
        .filter(|e| e.pubkey != input.self_pubkey)
        .filter(|e| {
            input.session.is_none_or(|session| {
                store
                    .session_membership_admits_event(&session.pubkey, channel, &e.id)
                    .unwrap_or(false)
            })
        })
        .filter(|e| !injected.contains(&e.id))
        .filter(|e| !is_backend_traffic(store, input.backend_pubkey, &e.pubkey, &e.tags_json))
        .filter_map(|ev| {
            let channel_ref = crate::channel_ref::full_channel_ref(store, &ev.channel_h);
            if channel_ref.is_empty() {
                return None;
            }
            let resolved_body = crate::profile::rewrite_body_mentions(store, &ev.content);
            let (body, truncated) = truncate_words(&resolved_body, CHAT_RENDER_WORD_LIMIT);
            Some(EvCap {
                id: ev.id.clone(),
                channel_ref,
                from_ref: pubkey_ref(store, &ev.pubkey, input.local_host),
                recipient_refs: p_tag_refs(store, &ev.tags_json, input.local_host),
                created_at: ev.created_at,
                body,
                truncated,
                mentions_self: mentions_pubkey(&ev.tags_json, input.self_pubkey),
                forced_mention: false,
                needs_reply_nudge: reply_nudge_for_message(
                    store,
                    input.self_pubkey,
                    &ev.channel_h,
                    &ev.id,
                    ev.created_at,
                    mentions_pubkey(&ev.tags_json, input.self_pubkey),
                ),
            })
        })
        .collect();
    let forced = forced
        .iter()
        .filter_map(|row| {
            let channel_ref = crate::channel_ref::full_channel_ref(store, channel);
            if channel_ref.is_empty() {
                return None;
            }
            let (body, truncated) = truncate_words(&row.body, CHAT_RENDER_WORD_LIMIT);
            Some(EvCap {
                id: row.id.clone(),
                channel_ref,
                from_ref: pubkey_ref(store, &row.from_pubkey, input.local_host),
                recipient_refs: forced_recipient_refs(store, input, row.mention),
                created_at: row.created_at,
                body,
                truncated,
                mentions_self: false,
                forced_mention: row.mention,
                needs_reply_nudge: reply_nudge_for_message(
                    store,
                    input.self_pubkey,
                    &row.channel,
                    &row.id,
                    row.created_at,
                    row.mention,
                ),
            })
        })
        .collect();
    MsgBundle { events, forced }
}

fn p_tag_refs(store: &Store, tags_json: &str, local_host: &str) -> Vec<String> {
    p_tag_pubkeys(tags_json)
        .into_iter()
        .map(|pk| pubkey_ref(store, &pk, local_host))
        .collect()
}

fn forced_recipient_refs(
    store: &Store,
    input: &FabricContextInput<'_>,
    mention: bool,
) -> Vec<String> {
    if !mention || input.self_pubkey.is_empty() {
        return Vec::new();
    }
    vec![pubkey_ref(store, input.self_pubkey, input.local_host)]
}

pub(super) fn resolve_pubkey(
    store: &Store,
    pubkey: &str,
    local_host: &str,
    refs: &mut BTreeMap<String, String>,
    agent_slugs: &mut BTreeMap<String, String>,
    backend: &mut BTreeSet<String>,
    has_handle: &mut BTreeSet<String>,
) {
    if refs.contains_key(pubkey) {
        return;
    }
    refs.insert(pubkey.to_string(), pubkey_ref(store, pubkey, local_host));
    if let Some(profile) = store.get_profile(pubkey).ok().flatten() {
        if !profile.slug.trim().is_empty() {
            has_handle.insert(pubkey.to_string());
        }
        if !profile.agent_slug.is_empty() {
            agent_slugs.insert(pubkey.to_string(), profile.agent_slug);
        }
        if profile.is_backend {
            backend.insert(pubkey.to_string());
        }
    }
}

fn channel_readiness(store: &Store, channel: &str) -> ChannelReadiness {
    match store.get_channel(channel) {
        Ok(Some(ch)) if !ch.is_archived() => ChannelReadiness::Ready,
        Ok(Some(_)) => ChannelReadiness::Archived,
        _ => ChannelReadiness::Missing,
    }
}

fn reply_nudge_for_message(
    store: &Store,
    self_pubkey: &str,
    channel_h: &str,
    message_id: &str,
    created_at: u64,
    mention: bool,
) -> bool {
    if !mention || self_pubkey.is_empty() {
        return false;
    }
    store
        .should_render_reply_nudge(channel_h, message_id, self_pubkey, created_at)
        .unwrap_or(true)
}
