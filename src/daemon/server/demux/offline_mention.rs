use super::super::resolution::work_root_for;
use super::super::*;
use std::sync::Arc;

mod claim;
mod headless;
pub(super) mod liveness;

pub(super) use claim::dispatch_all;
use headless::{mention_prompt, spawn_headless_mention};
use liveness::has_alive_session_for;

/// Spawn a local agent that was p-tagged in a kind:9 message but had no alive
/// session. The caller durably claims `(event_id, mentioned_pubkey)` before
/// entering this handler, so relay replay cannot repeat the side effect after a
/// daemon restart. `has_alive` still avoids unnecessary work on the first sight.
/// Delivery: session start schedules subscription/replay work in the daemon;
/// recent kind:9 events are re-materialized against the now-alive session and
/// delivered via `ring_doorbells`.
pub(super) async fn handle(
    state: &Arc<DaemonState>,
    mentioned_pk: &str,
    channel: &str,
    body: &str,
    requester_pubkey: Option<&str>,
) {
    let has_alive = state.with_store(|s| has_alive_session_for(s, mentioned_pk, channel));
    if has_alive {
        tracing::debug!(
            mentioned_pk = %crate::util::pubkey_short(mentioned_pk),
            channel,
            "agent already has alive session - skipping spawn"
        );
        return;
    }

    let now = now_secs();
    let backend_pubkey = state.backend_pubkey();
    let claim = state
        .with_store(|s| s.get_session_claim(mentioned_pk, channel).ok().flatten())
        .filter(|c| c.is_owned_by_backend(backend_pubkey.as_deref()));
    let active_claim = state.with_store(|s| {
        s.get_active_session_claim(mentioned_pk, channel, now)
            .ok()
            .flatten()
    });
    if let Some(remote_claim) = active_claim
        .as_ref()
        .filter(|c| !c.is_owned_by_backend(backend_pubkey.as_deref()))
    {
        tracing::info!(
            agent = %remote_claim.agent_slug,
            channel,
            owner_host = %remote_claim.owner_host,
            owner_backend = %crate::util::pubkey_short(&remote_claim.owner_backend_pubkey),
            "active ephemeral session claim belongs to another backend - skipping local spawn"
        );
        return;
    }
    let active_claim = active_claim.filter(|c| c.is_owned_by_backend(backend_pubkey.as_deref()));
    let profile_slug = state.with_store(|s| {
        s.get_profile(mentioned_pk)
            .ok()
            .flatten()
            .and_then(|profile| (!profile.agent_slug.is_empty()).then_some(profile.agent_slug))
    });
    let Some(agent_slug) = active_claim
        .as_ref()
        .or(claim.as_ref())
        .map(|route| route.agent_slug.clone())
        .filter(|slug| !slug.is_empty())
        .or(profile_slug)
    else {
        return;
    };

    let work_root = state.with_store(|s| work_root_for(s, channel));
    let has_path = state.with_store(|s| s.workspace_path(&work_root).ok().flatten().is_some());
    if !has_path {
        tracing::warn!(agent = %agent_slug, work_root = %work_root, channel, "no local channel root found - cannot spawn");
        return;
    }

    // A dormant claim is a route hint, not a resumable identity. Fresh launch
    // allocates the next session pubkey before the child starts.
    tracing::info!(agent = %agent_slug, channel, work_root = %work_root, "spawning agent on mention");
    match spawn_headless_mention(
        state,
        &agent_slug,
        &work_root,
        channel,
        body,
        notice_context(state, mentioned_pk, &agent_slug, requester_pubkey),
    )
    .await
    {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(agent = %agent_slug, channel, error = %e, "headless spawn failed - falling back to PTY spawn");
        }
    }
    match crate::session_host::spawn_ephemeral_agent(
        state,
        &agent_slug,
        &work_root,
        Some(channel),
        None,
    )
    .await
    {
        Ok(pty_id) => {
            tracing::info!(agent = %agent_slug, pty_id = %pty_id, channel, "agent spawned successfully");
            inject_spawn_prompt(agent_slug.clone(), pty_id, body);
        }
        Err(e) => {
            tracing::warn!(agent = %agent_slug, channel, error = %e, "agent spawn failed");
            headless::publish_start_failure_notice(
                state,
                &agent_slug,
                &target_label(state, mentioned_pk, &agent_slug),
                channel,
                requester_pubkey,
                &e.to_string(),
            )
            .await;
        }
    }
}

fn target_label(state: &Arc<DaemonState>, pubkey: &str, fallback: &str) -> String {
    state
        .with_store(|s| {
            s.get_profile(pubkey)
                .ok()
                .flatten()
                .and_then(|p| (!p.name.is_empty()).then_some(p.name))
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn notice_context(
    state: &Arc<DaemonState>,
    pubkey: &str,
    fallback: &str,
    requester_pubkey: Option<&str>,
) -> headless::MentionNotice {
    headless::MentionNotice {
        requester_pubkey: requester_pubkey.map(str::to_string),
        target_label: Some(target_label(state, pubkey, fallback)),
    }
}

fn inject_spawn_prompt(agent: String, pty_id: String, body: &str) {
    let prompt = mention_prompt(body);
    // Transport-aware: an ACP spawn's child lives in the daemon registry and is
    // driven over JSON-RPC; a PTY spawn is bracketed-paste injected. The endpoint
    // id (`pty_id`) is resolved through the transport's typed locator.
    // `deliver_spawn_prompt` logs
    // its own failures, so the fresh-spawn mention is never silently PTY-only.
    tokio::spawn(async move {
        crate::session_host::deliver_spawn_prompt(&agent, &pty_id, &prompt).await;
    });
}
