//! The ONE shared channel name→id resolver.
//!
//! The identity of a channel is the `(parent, name)` pair; the `channel_h` is its
//! durable, opaque key. This module is the single place a human channel NAME first
//! becomes a wire NIP-29 `h`, so every downstream consumer (launch/provision,
//! session-start, chat, switch) lands on ONE id and the old "name vs id" double
//! create can never recur.

use super::*;

mod paths;
pub(in crate::daemon::server) use paths::channel_reference_for;
use paths::{
    canonical_channel_reference, canonical_segments, path_ends_with, subtree_ids, subtree_paths,
};

/// Resolve `name` to a `channel_h` using ONLY locally-known state — no minting,
/// no relay calls. Returns `Some(h)` when the value resolves without provisioning:
///   1. An existing `(parent, name)` row wins (the durable key for that handle).
///
/// Returns `None` only for a genuine human NAME with no local row — the caller
/// then mints (when `create_if_absent`) or bails.
fn resolve_locally(
    store: &crate::state::Store,
    parent: &str,
    name: &str,
) -> Result<Option<String>> {
    // The channel root's `channel_h` IS its slug and it has no parent, so a request
    // to resolve `name` under a `parent` equal to it is the root asking for ITSELF —
    // return it unchanged, never mint a child of the root literally named after the
    // root. This is the load-bearing cold-cache case: a bare `launch` (no --channel)
    // scopes the session to the channel root by passing the slug as both work-root
    // and channel; right after a state/relay reset the root's kind:39000 has not yet
    // materialized, so checks 2–3 below miss and, without this guard, an opaque
    // child (parent=slug, name=slug) gets minted — the name-vs-id double-create.
    if parent == name {
        return Ok(Some(name.to_string()));
    }
    if let Some(h) = store.channel_id_for_name(parent, name)? {
        return Ok(Some(h));
    }
    Ok(None)
}

/// Resolve a channel NAME to its opaque `channel_h` within `parent`.
///
/// Local resolution (see [`resolve_locally`]) runs first: an existing
/// `(parent, name)` row.
/// Otherwise, when `create_if_absent`, mint exactly ONE opaque id and provision
/// it exactly like `channel_create` does (upsert + ready + sub); else bail — no
/// silent literal-`h` mint.
///
/// Channel resolution provisions with the management key only. The eventual
/// session signer is selected later by `session_start`; pre-adding the base
/// agent pubkey here would make the roster-aware ordinal allocator think the
/// first session is already occupied and incorrectly spawn `agent1`.
pub(in crate::daemon::server) async fn resolve_channel(
    state: &Arc<DaemonState>,
    parent: &str,
    name: &str,
    _agent: Option<&str>,
    create_if_absent: bool,
) -> Result<String> {
    if let Some(h) = state.with_store(|s| resolve_locally(s, parent, name))? {
        return Ok(h);
    }
    if !create_if_absent {
        anyhow::bail!("channel {name} not found");
    }

    let proposed = crate::util::opaque_group_id();
    let child_h = state
        .with_store(|s| s.reserve_channel_resolution_intent(parent, name, &proposed, now_secs()))?;
    let member = state.backend_pubkey().unwrap_or_default();

    let gate = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: &child_h,
            expect_member: &member,
            parent_hint: Some(parent),
            // Operator-chosen name rides on the create publish; the relay's
            // kind:39000 echo lands it in the cache.
            name: Some(name),
            repair_whitelisted_admins: true,
        })
        .await;
    // Fail loud: the relay never confirmed the new channel (its kind:39000 did not
    // materialize), so there is no real id to hand back. Returning `child_h` here
    // would point callers at a channel with no `relay_channels` row — exactly the
    // phantom-state the relay-sourced rule forbids.
    if matches!(gate, crate::fabric::nip29::readiness::ChannelGate::Degraded) {
        anyhow::bail!(
            "relay did not provision channel {name:?} (id {child_h}, parent {parent}); \
             its kind:39000 never materialized"
        );
    }
    if let Err(e) = ensure_subscription(state, &child_h).await {
        tracing::warn!(
            channel = %child_h,
            error = %format!("{e:#}"),
            "resolve_channel: ensure_subscription failed — new channel events may not be delivered until the next subscription sync"
        );
    }
    Ok(child_h)
}

/// Resolve a dotted path within `root`, provisioning each missing ancestor via
/// [`resolve_channel`] when requested. There is no depth cap.
pub(in crate::daemon::server) async fn resolve_channel_path(
    state: &Arc<DaemonState>,
    root: &str,
    reference: &str,
    create_if_absent: bool,
) -> Result<String> {
    if reference.contains('/') {
        anyhow::bail!("channel paths use dots, not slashes: {reference:?}");
    }
    let segments = canonical_segments(root, reference);
    if segments.is_empty() {
        return Ok(root.to_string());
    }
    let mut parent = root.to_string();
    for seg in &segments {
        if parent == root && seg.eq_ignore_ascii_case(root) {
            anyhow::bail!("{seg} is already the workspace root channel; use {root:?} instead");
        }
        parent = resolve_channel(state, &parent, seg, None, create_if_absent).await?;
    }
    Ok(parent)
}

/// `channel_resolve` RPC: thin wrapper over [`resolve_channel`] so the CLI launch
/// path can convert `--channel <name>` to its opaque id BEFORE spawning the PTY,
/// minting at most one group. Returns `{ channel_h }`.
pub(in crate::daemon::server) async fn rpc_channel_resolve(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
        name: String,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        create_if_absent: bool,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_resolve params")?;
    let channel_h = resolve_channel(
        state,
        &p.channel,
        &p.name,
        p.agent.as_deref(),
        p.create_if_absent,
    )
    .await?;
    Ok(serde_json::json!({ "channel_h": channel_h }))
}

// ── channel-relative `channel switch` resolution ─────────────────────────────
//
// `channel switch` (an AGENT-only gesture) resolves a channel-RELATIVE reference
// to one opaque `channel_h`. There is no cross-channel switch — references are
// scoped to the current channel's subtree. On ambiguity the daemon returns the
// candidate paths so the agent re-runs with an exact one (a structured error,
// never an interactive prompt a hooks-only agent cannot answer).

/// Outcome of resolving a channel-relative channel reference.
pub(in crate::daemon::server) enum ChannelResolution {
    /// Exactly one channel matched → its opaque `channel_h`.
    Unique(String),
    /// Several suffix matches → their canonical full paths, sorted.
    Ambiguous(Vec<String>),
    /// Nothing in the channel subtree matched.
    NotFound,
}

/// Walk `parent` links up from `channel` to the top-level channel root.
pub(in crate::daemon::server) fn root_channel(
    store: &crate::state::Store,
    channel: &str,
) -> Result<String> {
    crate::daemon::workspace_path::WorkspacePathResolver::new(store).root_for_channel(channel)
}

/// Resolve a channel-relative `reference` within `root`'s subtree. Forms:
///   - `@<id-prefix>` → the channel whose opaque id starts with the prefix;
///   - `name` / `parent/child` → suffix-matched against descendant NAME paths
///     (the shortest unique suffix resolves; a full path disambiguates deeper).
pub(in crate::daemon::server) fn resolve_channel_ref(
    store: &crate::state::Store,
    root: &str,
    reference: &str,
) -> ChannelResolution {
    let reference = reference.trim();
    if reference.is_empty() {
        return ChannelResolution::NotFound;
    }
    let want = canonical_segments(root, reference)
        .into_iter()
        .map(|segment| segment.to_lowercase())
        .collect::<Vec<_>>();
    if want.is_empty() {
        return ChannelResolution::Unique(root.to_string());
    }
    let paths = subtree_paths(store, root);

    // Canonical explicit id reference — match by opaque id prefix across the subtree.
    if let Some(prefix) = reference.strip_prefix('@') {
        if prefix.is_empty() {
            return ChannelResolution::NotFound;
        }
        let mut hits = subtree_ids(store, root)
            .into_iter()
            .filter(|id| id.starts_with(prefix))
            .collect::<Vec<_>>();
        hits.sort();
        hits.dedup();
        return match hits.len() {
            0 => ChannelResolution::NotFound,
            1 => ChannelResolution::Unique(hits.remove(0)),
            _ => {
                ChannelResolution::Ambiguous(hits.into_iter().map(|id| format!("@{id}")).collect())
            }
        };
    }

    // Name path: suffix-match dotted requested segments against each
    // descendant's relative NAME path (case-insensitive).
    let hits: Vec<(String, Vec<String>)> = paths
        .into_iter()
        .filter(|(_, segs)| path_ends_with(segs, &want))
        .collect();
    finish_resolution(hits, root)
}

/// Reduce raw `(channel_h, name_path)` hits to a [`ChannelResolution`]: dedup by
/// id, then unique / ambiguous / none. The schema guarantees sibling names are
/// unique, so ambiguous suffix matches always render as distinct canonical paths.
fn finish_resolution(mut hits: Vec<(String, Vec<String>)>, root: &str) -> ChannelResolution {
    hits.sort();
    hits.dedup_by(|a, b| a.0 == b.0);
    match hits.len() {
        0 => ChannelResolution::NotFound,
        1 => ChannelResolution::Unique(hits.remove(0).0),
        _ => {
            let mut refs: Vec<String> = hits
                .iter()
                .map(|(_, segs)| canonical_channel_reference(root, segs))
                .collect();
            refs.sort();
            ChannelResolution::Ambiguous(refs)
        }
    }
}

#[cfg(test)]
mod tests;
