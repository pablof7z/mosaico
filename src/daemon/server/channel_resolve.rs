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
///      backward-compat for callers passing a literal id (tmux_resume re-scope,
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

    let child_h = crate::util::opaque_group_id();
    let member = state
        .cfg
        .management_nsec()
        .and_then(|n| nostr_sdk::prelude::Keys::parse(n).ok())
        .map(|k| k.public_key().to_hex())
        .unwrap_or_default();

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

/// `channels_resolve` RPC: thin wrapper over [`resolve_channel`] so the CLI launch
/// path can convert `--channel <name>` to its opaque id BEFORE spawning the pane,
/// minting at most one group. Returns `{ channel_h }`.
pub(in crate::daemon::server) async fn rpc_channels_resolve(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        name: String,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        create_if_absent: bool,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_resolve params")?;
    let channel_h = resolve_channel(
        state,
        &p.project,
        &p.name,
        p.agent.as_deref(),
        p.create_if_absent,
    )
    .await?;
    Ok(serde_json::json!({ "channel_h": channel_h }))
}

// ── project-relative `channels switch` resolution ─────────────────────────────
//
// `channels switch` (an AGENT-only gesture) resolves a project-RELATIVE reference
// to one opaque `channel_h`. There is no cross-project switch — references are
// scoped to the current project's subtree. On ambiguity the daemon returns the
// candidate paths so the agent re-runs with an exact one (a structured error,
// never an interactive prompt a hooks-only agent cannot answer).

/// Outcome of resolving a project-relative channel reference.
pub(in crate::daemon::server) enum ChannelResolution {
    /// Exactly one channel matched → its opaque `channel_h`.
    Unique(String),
    /// Several matched → the exact re-run references (a unique relative path, or
    /// the `@<id>` escape hatch when two siblings share a name), sorted.
    Ambiguous(Vec<String>),
    /// Nothing in the project subtree matched.
    NotFound,
}

/// Walk `parent` links up from `channel` to the top-level project root.
pub(in crate::daemon::server) fn project_root(
    store: &crate::state::Store,
    channel: &str,
) -> String {
    store
        .channel_project_root(channel)
        .unwrap_or_else(|e| {
            tracing::error!(
                channel = %channel,
                error = %e,
                "project_root: channel ancestry lookup failed"
            );
            None
        })
        .unwrap_or_else(|| channel.to_string())
}

/// Resolve a project-relative `reference` within `root`'s subtree. Forms:
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
    // Id passthrough: a caller (tmux resume, launch picker) may pass a literal id.
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
    // relative NAME path (case-insensitive).
    let want: Vec<String> = reference
        .split('/')
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
mod resolve_tests {
    use super::{project_root, resolve_channel_ref, resolve_locally, ChannelResolution};
    use crate::state::Store;

    fn chan(store: &Store, id: &str, name: &str, parent: &str) {
        store.upsert_channel(id, name, "", parent, 1).unwrap();
    }

    /// An 8-hex opaque id absent from the local cache (a freshly provisioned
    /// channel whose kind:39000 hasn't materialized yet) resolves to ITSELF and
    /// does NOT mint a literal-named channel — the launch-channel-scope fix.
    #[test]
    fn opaque_id_miss_passes_through_without_minting() {
        let store = Store::open_memory().unwrap();
        chan(&store, "h-root", "proj", "");
        let id = "2f1cd36f";
        assert!(
            store.get_channel(id).unwrap().is_none(),
            "precondition: opaque id must be absent from the cache"
        );
        assert_eq!(
            resolve_locally(&store, "h-root", id).unwrap(),
            Some(id.to_string()),
            "an unknown opaque id must pass through unchanged, not be minted"
        );
        // Passthrough is pure: it must not have created a channel named after the id.
        assert!(
            store.get_channel(id).unwrap().is_none(),
            "resolve_locally must never mint a channel"
        );
    }

    /// Known names/ids resolve locally; a genuine human NAME with no row does NOT
    /// (the caller mints/bails) — proving the opaque-id passthrough never
    /// over-triggers on real handles.
    #[test]
    fn known_resolve_locally_but_unknown_human_name_does_not() {
        let store = Store::open_memory().unwrap();
        chan(&store, "h-root", "proj", "");
        chan(&store, "h-plan", "planning", "h-root");
        // 1. existing (parent, name) row wins.
        assert_eq!(
            resolve_locally(&store, "h-root", "planning").unwrap(),
            Some("h-plan".to_string())
        );
        // 2. a literal known channel_h passes through.
        assert_eq!(
            resolve_locally(&store, "h-root", "h-plan").unwrap(),
            Some("h-plan".to_string())
        );
        // 3. a genuine human NAME with no local row is unresolved here.
        assert_eq!(
            resolve_locally(&store, "h-root", "backlog-work").unwrap(),
            None
        );
    }

    #[test]
    fn unique_relative_name_resolves() {
        let store = Store::open_memory().unwrap();
        chan(&store, "h-root", "proj", "");
        chan(&store, "h-plan", "planning", "h-root");
        match resolve_channel_ref(&store, "h-root", "planning") {
            ChannelResolution::Unique(id) => assert_eq!(id, "h-plan"),
            _ => panic!("expected unique match"),
        }
    }

    #[test]
    fn ambiguous_name_lists_relative_paths() {
        let store = Store::open_memory().unwrap();
        chan(&store, "h-root", "proj", "");
        chan(&store, "h-plan", "planning", "h-root");
        chan(&store, "h-epic", "epic999", "h-root");
        chan(&store, "h-epic-plan", "planning", "h-epic");
        match resolve_channel_ref(&store, "h-root", "planning") {
            ChannelResolution::Ambiguous(refs) => {
                assert_eq!(
                    refs,
                    vec!["epic999/planning".to_string(), "planning".to_string()]
                );
            }
            _ => panic!("expected ambiguous"),
        }
        // A fuller path disambiguates.
        assert!(matches!(
            resolve_channel_ref(&store, "h-root", "epic999/planning"),
            ChannelResolution::Unique(ref id) if id == "h-epic-plan"
        ));
    }

    #[test]
    fn same_level_name_collision_falls_back_to_id_escape_hatch() {
        let store = Store::open_memory().unwrap();
        chan(&store, "h-root", "proj", "");
        // Two siblings share the name "planning" — a path cannot disambiguate.
        chan(&store, "h-aaaa1111", "planning", "h-root");
        chan(&store, "h-bbbb2222", "planning", "h-root");
        match resolve_channel_ref(&store, "h-root", "planning") {
            ChannelResolution::Ambiguous(refs) => {
                assert_eq!(refs, vec!["@h-aaaa11".to_string(), "@h-bbbb22".to_string()]);
            }
            _ => panic!("expected ambiguous id-escape-hatch"),
        }
        // The @id escape hatch then resolves uniquely.
        assert!(matches!(
            resolve_channel_ref(&store, "h-root", "@h-aaaa1"),
            ChannelResolution::Unique(ref id) if id == "h-aaaa1111"
        ));
    }

    #[test]
    fn literal_id_passthrough_and_not_found() {
        let store = Store::open_memory().unwrap();
        chan(&store, "h-root", "proj", "");
        chan(&store, "h-plan", "planning", "h-root");
        assert!(matches!(
            resolve_channel_ref(&store, "h-root", "h-plan"),
            ChannelResolution::Unique(ref id) if id == "h-plan"
        ));
        assert!(matches!(
            resolve_channel_ref(&store, "h-root", "nonexistent"),
            ChannelResolution::NotFound
        ));
    }

    #[test]
    fn nested_sender_explicit_channel_refs_resolve_from_project_root() {
        let store = Store::open_memory().unwrap();
        chan(&store, "h-root", "proj", "");
        chan(&store, "h-epic", "epic", "h-root");
        chan(&store, "h-plan", "planning", "h-epic");
        chan(&store, "h-leaf", "leaf", "h-plan");
        chan(&store, "h-review", "review", "h-epic");

        let root = project_root(&store, "h-leaf");
        assert_eq!(root, "h-root");
        assert!(matches!(
            resolve_channel_ref(&store, &root, "epic/review"),
            ChannelResolution::Unique(ref id) if id == "h-review"
        ));
    }
}
