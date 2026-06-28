//! The ONE shared channel name→id resolver.
//!
//! The identity of a channel is the `(parent, name)` pair; the `channel_h` is its
//! durable, opaque key. This module is the single place a human channel NAME first
//! becomes a wire NIP-29 `h`, so every downstream consumer (launch/provision,
//! session-start, chat, switch) lands on ONE id and the old "name vs id" double
//! create can never recur.

use super::*;

/// Resolve a channel NAME to its opaque `channel_h` within `parent`.
///
/// Order of resolution:
///   1. An existing `(parent, name)` row wins (the durable key for that handle).
///   2. A value that is ALREADY a known `channel_h` is returned unchanged —
///      backward-compat for callers passing a literal id (tmux_resume re-scope,
///      `channels switch`, a launch whose picker already returned an id).
///   3. Otherwise, when `create_if_absent`, mint exactly ONE opaque id and
///      provision it exactly like `channels_create` does (upsert + ready + sub).
///   4. Else bail — no silent literal-`h` mint.
///
/// `agent` (a slug) names the member to admit when a channel is minted; when
/// absent the management key (already the group admin) provisions it.
pub(in crate::daemon::server) async fn resolve_channel(
    state: &Arc<DaemonState>,
    parent: &str,
    name: &str,
    agent: Option<&str>,
    create_if_absent: bool,
) -> Result<String> {
    if let Some(h) = state.with_store(|s| s.channel_id_for_name(parent, name))? {
        return Ok(h);
    }
    // A literal channel_h already known locally is treated as already-resolved.
    if state
        .with_store(|s| s.get_channel(name))
        .ok()
        .flatten()
        .is_some()
    {
        return Ok(name.to_string());
    }
    if !create_if_absent {
        anyhow::bail!("channel {name} not found");
    }

    let child_h = crate::util::opaque_group_id();
    let now = now_secs();
    // Stamp the operator-chosen name + parent locally FIRST so the shared
    // provisioning primitive names the new subgroup correctly (it reads the
    // display name from the local store).
    state.with_store(|s| {
        s.upsert_channel(&child_h, name, "", parent, now).ok();
    });

    // The member to admit: the named agent's durable pubkey, else the management
    // key (already an admin) purely to provision the group.
    let member = match agent.filter(|a| !a.is_empty()) {
        Some(slug) => crate::identity::load_or_create(&crate::config::edge_home(), slug, now)
            .map(|id| id.pubkey_hex())
            .ok(),
        None => None,
    }
    .or_else(|| {
        state
            .cfg
            .management_nsec()
            .and_then(|n| nostr_sdk::prelude::Keys::parse(n).ok())
            .map(|k| k.public_key().to_hex())
    })
    .unwrap_or_default();

    let _ = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: &child_h,
            expect_member: &member,
            parent_hint: Some(parent),
        })
        .await;
    let _ = ensure_subscription(state, &child_h).await;
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
