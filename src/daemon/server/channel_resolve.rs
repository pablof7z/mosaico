//! The ONE shared channel name→id resolver.
//!
//! The identity of a channel is the `(parent, name)` pair; the `channel_h` is its
//! durable, opaque key. This module is the single place a human channel NAME first
//! becomes a wire NIP-29 `h`, so every downstream consumer (launch/provision,
//! session-start, chat, switch) lands on ONE id and the old "name vs id" double
//! create can never recur.

use super::*;

/// Resolve `name` to a `channel_h` using ONLY locally-known state — no minting,
/// no relay calls. Returns `Some(h)` when the value resolves without provisioning:
///   1. An existing `(parent, name)` row wins (the durable key for that handle).
///   2. A value that is ALREADY a known `channel_h` is returned unchanged —
///      backward-compat for callers passing a literal id (resume re-scope,
///      `channels switch`, a launch whose picker already returned an id).
///   3. A value SHAPED like an opaque id (`[0-9a-f]{8}`) that missed 1–2 is an
///      already-resolved id whose kind:39000 has not yet materialized into the
///      local cache (a race vs the channel's own provisioning) — hand it back
///      unchanged rather than minting a junk channel literally NAMED after the
///      id. Every spawn/launch sets `TENEX_EDGE_CHANNEL` to an already-resolved
///      opaque id, so this is the common case for a freshly provisioned channel.
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
    if store.get_channel(name).ok().flatten().is_some() {
        return Ok(Some(name.to_string()));
    }
    if crate::util::is_opaque_group_id(name) {
        return Ok(Some(name.to_string()));
    }
    Ok(None)
}

pub(in crate::daemon::server) struct SessionStartChannelResolution {
    pub channel_h: String,
    pub provision_name: Option<String>,
}

pub(in crate::daemon::server) fn resolve_channel_for_session_start(
    state: &Arc<DaemonState>,
    parent: &str,
    name: &str,
) -> Result<SessionStartChannelResolution> {
    if let Some(h) = state.with_store(|s| resolve_locally(s, parent, name))? {
        return Ok(SessionStartChannelResolution {
            channel_h: h,
            provision_name: None,
        });
    }

    let proposed = crate::util::opaque_group_id();
    let channel_h = state
        .with_store(|s| s.reserve_channel_resolution_intent(parent, name, &proposed, now_secs()))?;
    Ok(SessionStartChannelResolution {
        channel_h,
        provision_name: Some(name.to_string()),
    })
}

/// Resolve a channel NAME to its opaque `channel_h` within `parent`.
///
/// Local resolution (see [`resolve_locally`]) runs first: an existing
/// `(parent, name)` row, a known `channel_h`, or an opaque-id passthrough.
/// Otherwise, when `create_if_absent`, mint exactly ONE opaque id and provision
/// it exactly like `channels_create` does (upsert + ready + sub); else bail — no
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
            "resolve_channel: ensure_subscription failed — new channel events may not be delivered until the next resubscribe"
        );
    }
    Ok(child_h)
}

/// `mkdir -p` for channel paths: resolve a channel-relative NAME path within
/// `root`, provisioning EVERY missing ancestor (not just the leaf) when
/// `create_if_absent`. Segments split on both `/` and `.`, so `a.b.c` and
/// `a/b/c` create the same three-deep chain. Each segment resolves as a child of
/// the previous via [`resolve_channel`]; a missing segment is minted and
/// provisioned (upsert + ready + sub) and becomes the parent for the next. There
/// is no depth cap — arbitrarily deep chains provision one level at a time.
pub(in crate::daemon::server) async fn resolve_channel_path(
    state: &Arc<DaemonState>,
    root: &str,
    reference: &str,
    create_if_absent: bool,
) -> Result<String> {
    let segments: Vec<String> = reference
        .split(['/', '.'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        anyhow::bail!("empty channel reference");
    }
    let mut parent = root.to_string();
    for seg in &segments {
        parent = resolve_channel(state, &parent, seg, None, create_if_absent).await?;
    }
    Ok(parent)
}

/// `channels_resolve` RPC: thin wrapper over [`resolve_channel`] so the CLI launch
/// path can convert `--channel <name>` to its opaque id BEFORE spawning the PTY,
/// minting at most one group. Returns `{ channel_h }`.
pub(in crate::daemon::server) async fn rpc_channels_resolve(
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
    let p: P = serde_json::from_value(params.clone()).context("channels_resolve params")?;
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

// ── channel-relative `channels switch` resolution ─────────────────────────────
//
// `channels switch` (an AGENT-only gesture) resolves a channel-RELATIVE reference
// to one opaque `channel_h`. There is no cross-channel switch — references are
// scoped to the current channel's subtree. On ambiguity the daemon returns the
// candidate paths so the agent re-runs with an exact one (a structured error,
// never an interactive prompt a hooks-only agent cannot answer).

/// Outcome of resolving a channel-relative channel reference.
pub(in crate::daemon::server) enum ChannelResolution {
    /// Exactly one channel matched → its opaque `channel_h`.
    Unique(String),
    /// Several matched → the exact re-run references (a unique relative path, or
    /// the `@<id>` escape hatch when two siblings share a name), sorted.
    Ambiguous(Vec<String>),
    /// Nothing in the channel subtree matched.
    NotFound,
}

/// Walk `parent` links up from `channel` to the top-level channel root.
pub(in crate::daemon::server) fn root_channel(
    store: &crate::state::Store,
    channel: &str,
) -> String {
    store
        .root_channel_of(channel)
        .unwrap_or_else(|e| {
            tracing::error!(
                channel = %channel,
                error = %e,
                "root_channel: channel ancestry lookup failed"
            );
            None
        })
        .unwrap_or_else(|| channel.to_string())
}

/// Resolve a channel-relative `reference` within `root`'s subtree. Forms:
///   - a literal known `channel_h` → returned unchanged (id passthrough);
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
    // Id passthrough: a caller (resume, launch picker) may pass a literal id.
    if store.get_channel(reference).ok().flatten().is_some() {
        return ChannelResolution::Unique(reference.to_string());
    }
    let paths = subtree_paths(store, root);

    // `@<prefix>` escape hatch — match by opaque id prefix across the subtree.
    if let Some(prefix) = reference.strip_prefix('@') {
        if prefix.is_empty() {
            return ChannelResolution::NotFound;
        }
        let hits: Vec<(String, Vec<String>)> = paths
            .iter()
            .filter(|(id, _)| id.starts_with(prefix))
            .map(|(id, segs)| (id.clone(), segs.clone()))
            .collect();
        return finish_resolution(hits);
    }

    // Name path: suffix-match the requested segments against each descendant's
    // relative NAME path (case-insensitive). Both `/` and `.` delimit path
    // segments, so `a/b/c` and `a.b.c` resolve identically.
    let want: Vec<String> = reference
        .split(['/', '.'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if want.is_empty() {
        return ChannelResolution::NotFound;
    }
    let hits: Vec<(String, Vec<String>)> = paths
        .into_iter()
        .filter(|(_, segs)| path_ends_with(segs, &want))
        .collect();
    finish_resolution(hits)
}

/// Reduce raw `(channel_h, name_path)` hits to a [`ChannelResolution`]: dedup by
/// id, then unique / ambiguous / none. Ambiguous entries render as their unique
/// relative path, or `@<id-prefix>` when two share the same path (a same-level
/// name collision that only the id can disambiguate).
fn finish_resolution(mut hits: Vec<(String, Vec<String>)>) -> ChannelResolution {
    hits.sort();
    hits.dedup_by(|a, b| a.0 == b.0);
    match hits.len() {
        0 => ChannelResolution::NotFound,
        1 => ChannelResolution::Unique(hits.remove(0).0),
        _ => {
            let mut refs: Vec<String> = hits
                .iter()
                .map(|(id, segs)| {
                    let path = segs.join("/");
                    let path_unique = hits.iter().filter(|(_, s)| s.join("/") == path).count() == 1;
                    if path_unique && !path.is_empty() {
                        path
                    } else {
                        format!("@{}", &id[..id.len().min(8)])
                    }
                })
                .collect();
            refs.sort();
            ChannelResolution::Ambiguous(refs)
        }
    }
}

/// Every channel in `root`'s subtree (excluding root) as `(channel_h, name_path)`,
/// where `name_path` is the chain of kind:39000 NAMES from root's child down to
/// the channel. Unnamed nodes (per [`Channel::human_name`] — e.g. session rooms
/// whose name defaulted to their opaque id) are not path-referenceable, so they
/// and their subtrees are skipped.
fn subtree_paths(store: &crate::state::Store, root: &str) -> Vec<(String, Vec<String>)> {
    use std::collections::BTreeMap;
    let channels = store.list_channels().unwrap_or_default();
    let mut by_parent: BTreeMap<String, Vec<crate::state::Channel>> = BTreeMap::new();
    for c in channels {
        by_parent.entry(c.parent.clone()).or_default().push(c);
    }
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut stack: Vec<(String, Vec<String>)> = vec![(root.to_string(), Vec::new())];
    let mut guard = 0usize;
    while let Some((id, path)) = stack.pop() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        let Some(children) = by_parent.get(&id) else {
            continue;
        };
        for c in children {
            let Some(name) = c.human_name() else {
                continue; // unnamed → not referenceable by path; skip its subtree
            };
            let mut child_path = path.clone();
            child_path.push(name.to_lowercase());
            out.push((c.channel_h.clone(), child_path.clone()));
            stack.push((c.channel_h.clone(), child_path));
        }
    }
    out
}

/// True when `segs` ends with `want` (both already lowercased), i.e. `want` is a
/// path suffix of `segs`. `["epic999","planning"]` ends with `["planning"]`.
fn path_ends_with(segs: &[String], want: &[String]) -> bool {
    segs.len() >= want.len() && segs[segs.len() - want.len()..] == *want
}

#[cfg(test)]
mod tests;
