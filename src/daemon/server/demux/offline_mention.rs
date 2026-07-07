use super::super::resolution::work_root_for;
use super::super::*;
use std::sync::Arc;

mod headless;

use headless::{mention_prompt, spawn_headless_mention};

/// Spawn a local agent that was p-tagged in a kind:9 message but had no alive
/// session. Idempotency: `first_sight` prevents duplicate spawns within a run;
/// `has_alive` prevents re-spawn across restarts when the previous spawn registered.
/// Delivery: `rpc_session_start` calls `ensure_subscription`, which triggers a
/// relay replay of recent kind:9 events; those are re-materialized against the
/// now-alive session and delivered via `ring_doorbells`.
pub(super) async fn handle(
    state: &Arc<DaemonState>,
    mentioned_pk: &str,
    project: &str,
    body: &str,
) {
    let has_alive = state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .any(|rec| {
                rec.agent_pubkey == mentioned_pk
                    && s.is_session_joined_channel(&rec.session_id, project)
                        .unwrap_or(rec.channel_h == project)
            })
    });
    if has_alive {
        tracing::debug!(
            mentioned_pk = %crate::util::pubkey_short(mentioned_pk),
            project,
            "agent already has alive session - skipping spawn"
        );
        return;
    }

    let now = now_secs();
    let backend_pubkey = state.backend_pubkey();
    let claim = state
        .with_store(|s| s.get_session_claim(mentioned_pk, project).ok().flatten())
        .filter(|c| c.is_owned_by_backend(backend_pubkey.as_deref()));
    let active_claim = state.with_store(|s| {
        s.get_active_session_claim(mentioned_pk, project, now)
            .ok()
            .flatten()
    });
    if let Some(remote_claim) = active_claim
        .as_ref()
        .filter(|c| !c.is_owned_by_backend(backend_pubkey.as_deref()))
    {
        tracing::info!(
            agent = %remote_claim.agent_slug,
            project,
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
            project,
            native_id = %route.native_id,
            "resuming active ephemeral session claim"
        );
        let resume_work_root = state.with_store(|s| work_root_for(s, project));
        match spawn_headless_mention(
            state,
            &route.agent_slug,
            &resume_work_root,
            project,
            body,
            Some(&route.native_id),
            None,
        )
        .await
        {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(agent = %route.agent_slug, project, error = %e, "headless resume failed - falling back to PTY resume");
            }
        }
        if let Err(e) =
            crate::session_host::resume_agent(state, &route.agent_slug, project, &route.native_id)
                .await
        {
            tracing::warn!(agent = %route.agent_slug, project, error = %e, "session resume failed - falling through to fresh spawn");
        } else {
            return;
        }
    }

    let identity = state.with_store(|s| {
        s.get_identity_for_channel(mentioned_pk, project)
            .ok()
            .flatten()
            .or_else(|| s.get_identity(mentioned_pk).ok().flatten())
    });
    let Some((agent_slug, ordinal)) = identity
        .map(|idn| (idn.agent_slug, idn.ordinal))
        .or_else(|| claim.as_ref().map(|c| (c.agent_slug.clone(), c.ordinal)))
    else {
        return;
    };

    let preferred_ordinal = match claim.as_ref() {
        Some(_) => None,
        None => Some(ordinal),
    };

    if preferred_ordinal.is_some() {
        let is_member =
            state.with_store(|s| s.is_channel_member(project, mentioned_pk).unwrap_or(false));
        if !is_member {
            let (_, _, members) = state.provider.fetch_group_state(project).await;
            if !members.contains(mentioned_pk) {
                tracing::info!(agent = %agent_slug, ordinal, project, "provisioning ordinal pubkey into channel via mgmt key");
                if !state
                    .provider
                    .grant_member_confirmed(project, mentioned_pk)
                    .await
                    .is_confirmed()
                {
                    tracing::warn!(agent = %agent_slug, ordinal, project, "mgmt-key add_member was not confirmed - skipping spawn");
                    return;
                }
            }
        }
    }

    let work_root = state.with_store(|s| work_root_for(s, project));
    let has_path = state.with_store(|s| s.project_root(&work_root).ok().flatten().is_some());
    if !has_path {
        tracing::warn!(agent = %agent_slug, work_root = %work_root, project, "no local project root found - cannot spawn");
        return;
    }

    let group_arg = Some(project);
    tracing::info!(
        agent = %agent_slug,
        ordinal,
        project,
        work_root = %work_root,
        "spawning agent on mention"
    );
    match spawn_headless_mention(
        state,
        &agent_slug,
        &work_root,
        project,
        body,
        None,
        preferred_ordinal,
    )
    .await
    {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(agent = %agent_slug, project, error = %e, "headless spawn failed - falling back to PTY spawn");
        }
    }
    match crate::session_host::spawn_ephemeral_agent(
        state,
        &agent_slug,
        &work_root,
        Vec::new(),
        None,
        group_arg,
        None,
        preferred_ordinal,
    )
    .await
    {
        Ok(pty_id) => {
            tracing::info!(agent = %agent_slug, pty_id = %pty_id, project, "agent spawned successfully");
            if preferred_ordinal.is_none() {
                inject_spawn_prompt(agent_slug.clone(), project.to_string(), pty_id, body);
            }
        }
        Err(e) => tracing::warn!(agent = %agent_slug, project, error = %e, "agent spawn failed"),
    }
}

fn inject_spawn_prompt(agent: String, project: String, pty_id: String, body: &str) {
    let prompt = mention_prompt(body);
    tokio::spawn(async move {
        if let Err(e) = crate::session_host::inject_spawn_message(&pty_id, &prompt).await {
            tracing::warn!(agent = %agent, project = %project, pty_id = %pty_id, error = %e, "failed to inject mention prompt into fresh PTY spawn");
        }
    });
}
