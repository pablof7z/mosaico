use super::super::resolution::work_root_for;
use super::super::*;
use std::sync::Arc;

mod headless;

use headless::{mention_prompt, spawn_headless_mention};

pub(super) fn dispatch(
    state: &Arc<DaemonState>,
    chat: &crate::domain::ChatMessage,
    mentioned_pk: &str,
) {
    let st = state.clone();
    let mentioned_pk = mentioned_pk.to_string();
    let channel = chat.channel.clone();
    let body = chat.body.clone();
    let requester_pubkey = chat.from.pubkey.clone();
    tracing::info!(
        mentioned_pk = %crate::util::pubkey_short(&mentioned_pk),
        channel = %channel,
        "dispatching offline-agent-mention handler"
    );
    tokio::spawn(async move {
        handle(&st, &mentioned_pk, &channel, &body, Some(&requester_pubkey)).await;
    });
}

/// Spawn a local agent that was p-tagged in a kind:9 message but had no alive
/// session. Idempotency: `first_sight` prevents duplicate spawns within a run;
/// `has_alive` prevents re-spawn across restarts when the previous spawn registered.
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
    let has_alive = state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .any(|rec| {
                rec.agent_pubkey == mentioned_pk
                    && s.is_session_joined_channel(&rec.session_id, channel)
                        .unwrap_or(rec.channel_h == channel)
            })
    });
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
    let active_claim = active_claim
        .filter(|c| c.is_owned_by_backend(backend_pubkey.as_deref()))
        .filter(|c| !c.native_id.is_empty());

    if let Some(route) = active_claim.as_ref() {
        tracing::info!(
            agent = %route.agent_slug,
            channel,
            native_id = %route.native_id,
            "resuming active ephemeral session claim"
        );
        let resume_work_root = state.with_store(|s| work_root_for(s, channel));
        match spawn_headless_mention(
            state,
            &route.agent_slug,
            &resume_work_root,
            channel,
            body,
            Some(&route.native_id),
            notice_context(state, mentioned_pk, &route.agent_slug, requester_pubkey),
        )
        .await
        {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(agent = %route.agent_slug, channel, error = %e, "headless resume failed - falling back to PTY resume");
            }
        }
        if let Err(e) =
            crate::session_host::resume_agent(state, &route.agent_slug, channel, &route.native_id)
                .await
        {
            tracing::warn!(agent = %route.agent_slug, channel, error = %e, "session resume failed - falling through to fresh spawn");
        } else {
            return;
        }
    }

    // No live session and no active claim to resume. Look up the minted identity
    // bound to the p-tagged pubkey to learn which agent + native session it
    // belongs to. A per-session pubkey is unique, so this resolves to exactly one
    // session — resuming it (via its native id) reproduces the same pubkey.
    let identity = state.with_store(|s| {
        s.get_identity_for_channel(mentioned_pk, channel)
            .ok()
            .flatten()
            .or_else(|| s.get_identity(mentioned_pk).ok().flatten())
    });
    let Some((agent_slug, native_id)) = identity
        .map(|idn| (idn.agent_slug, idn.native_id))
        .or_else(|| {
            claim
                .as_ref()
                .map(|c| (c.agent_slug.clone(), c.native_id.clone()))
        })
    else {
        return;
    };

    let work_root = state.with_store(|s| work_root_for(s, channel));
    let has_path = state.with_store(|s| s.workspace_path(&work_root).ok().flatten().is_some());
    if !has_path {
        tracing::warn!(agent = %agent_slug, work_root = %work_root, channel, "no local channel root found - cannot spawn");
        return;
    }

    // If we know the native session id, resume THAT session so the resumed process
    // re-derives the p-tagged pubkey. Grant that pubkey channel membership up front
    // (via mgmt key) so its replayed events are accepted.
    if !native_id.is_empty() {
        let is_member =
            state.with_store(|s| s.is_channel_member(channel, mentioned_pk).unwrap_or(false));
        if !is_member {
            let (_, _, members) = state.provider.fetch_group_state(channel).await;
            if !members.contains(mentioned_pk) {
                tracing::info!(agent = %agent_slug, channel, "provisioning session pubkey into channel via mgmt key");
                if !state
                    .provider
                    .grant_member_confirmed(channel, mentioned_pk)
                    .await
                    .is_confirmed()
                {
                    tracing::warn!(agent = %agent_slug, channel, "mgmt-key add_member was not confirmed - skipping resume");
                    return;
                }
            }
        }
        tracing::info!(agent = %agent_slug, channel, native_id = %native_id, "resuming session on mention");
        match spawn_headless_mention(
            state,
            &agent_slug,
            &work_root,
            channel,
            body,
            Some(&native_id),
            notice_context(state, mentioned_pk, &agent_slug, requester_pubkey),
        )
        .await
        {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(agent = %agent_slug, channel, error = %e, "headless resume failed - falling back to PTY resume");
            }
        }
        match crate::session_host::resume_agent(state, &agent_slug, channel, &native_id).await {
            Ok(pty_id) => {
                tracing::info!(agent = %agent_slug, pty_id = %pty_id, channel, "session resumed on mention");
                return;
            }
            Err(e) => {
                tracing::warn!(agent = %agent_slug, channel, error = %e, "PTY resume failed - falling back to fresh spawn");
            }
        }
    }

    // Fresh spawn: a brand-new session (a new pubkey) that starts the agent and
    // gets the mention injected so it can respond.
    tracing::info!(agent = %agent_slug, channel, work_root = %work_root, "spawning agent on mention");
    match spawn_headless_mention(
        state,
        &agent_slug,
        &work_root,
        channel,
        body,
        None,
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
        Vec::new(),
        None,
        Some(channel),
        None,
    )
    .await
    {
        Ok(pty_id) => {
            tracing::info!(agent = %agent_slug, pty_id = %pty_id, channel, "agent spawned successfully");
            inject_spawn_prompt(agent_slug.clone(), channel.to_string(), pty_id, body);
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
                .or_else(|| {
                    s.get_identity(pubkey)
                        .ok()
                        .flatten()
                        .and_then(|i| (!i.codename.is_empty()).then_some(i.codename))
                })
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

fn inject_spawn_prompt(agent: String, channel: String, pty_id: String, body: &str) {
    let prompt = mention_prompt(body);
    tokio::spawn(async move {
        if let Err(e) = crate::session_host::inject_spawn_message(&pty_id, &prompt).await {
            tracing::warn!(agent = %agent, channel = %channel, pty_id = %pty_id, error = %e, "failed to inject mention prompt into fresh PTY spawn");
        }
    });
}
