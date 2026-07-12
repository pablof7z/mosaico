use super::resolve::{local_session, remote_session};
use super::wait::{wait_local_session_online, wait_remote_session_online};
use super::*;

pub(super) async fn invite_session(
    state: &Arc<DaemonState>,
    channel_h: &str,
    work_root: &str,
    selector: &str,
) -> Result<serde_json::Value> {
    if let Some(rec) = local_session(state, selector) {
        if rec.alive {
            if let Some(pty_id) = live_pty_for_session(state, &rec) {
                return pull_live_session(state, channel_h, &rec, &pty_id).await;
            }
            if state.with_store(|s| s.is_durable_agent_session(&rec.session_id))? {
                return pull_live_session(state, channel_h, &rec, "").await;
            }
        }
        return resume_local_session(state, channel_h, work_root, &rec).await;
    }

    invite_remote_session(state, channel_h, selector).await
}

async fn pull_live_session(
    state: &Arc<DaemonState>,
    channel_h: &str,
    rec: &crate::state::Session,
    pty_id: &str,
) -> Result<serde_json::Value> {
    ensure_live_session_member(state, channel_h, rec).await?;
    ensure_subscription(state, channel_h).await?;
    state.with_store(|s| s.join_session_channel(&rec.session_id, channel_h, now_secs()))?;
    let online = wait_local_session_online(state, channel_h, &rec.session_id).await?;
    Ok(serde_json::json!({
        "pty_id": pty_id,
        "npub": crate::idref::npub(&rec.agent_pubkey),
        "agent": rec.agent_slug,
        "online_agent": online,
        "channel": channel_h,
        "host": state.host,
    }))
}

async fn resume_local_session(
    state: &Arc<DaemonState>,
    channel_h: &str,
    work_root: &str,
    rec: &crate::state::Session,
) -> Result<serde_json::Value> {
    let resume_id = super::super::pty_rpc::resume_token_for(rec).with_context(|| {
        format!(
            "session {} has no resume token (not resumable)",
            rec.session_id
        )
    })?;
    super::super::pty_rpc::provision_before_spawn(
        state,
        &rec.agent_slug,
        work_root,
        Some(channel_h),
    )
    .await?;
    let pty_id = crate::session_host::resume_agent_in_channel(
        state,
        &rec.agent_slug,
        work_root,
        channel_h,
        &resume_id,
    )
    .await?;
    let online = wait_local_session_online(state, channel_h, &rec.session_id).await?;
    Ok(serde_json::json!({
        "pty_id": pty_id,
        "npub": crate::idref::npub(&rec.agent_pubkey),
        "agent": rec.agent_slug,
        "online_agent": online,
        "channel": channel_h,
        "host": state.host,
    }))
}

async fn invite_remote_session(
    state: &Arc<DaemonState>,
    channel_h: &str,
    selector: &str,
) -> Result<serde_json::Value> {
    let remote = remote_session(state, selector)?;
    if remote.backend == state.host {
        anyhow::bail!(
            "session {} appears to belong to this backend, but no local identity row exists",
            crate::idref::npub(&remote.pubkey).unwrap_or_else(|| remote.pubkey.clone())
        );
    }
    let backend_pubkey = resolve_backend_pubkey(state, &remote.backend).await?;
    super::ensure_backend_admin(state, channel_h, &backend_pubkey).await?;
    let event_id = super::publish_invite_orchestration(
        state,
        channel_h,
        crate::fabric::nip29::orchestration::AddTarget {
            backend_pubkey,
            slug: remote.slug.clone(),
            session_pubkey: Some(remote.pubkey.clone()),
        },
    )
    .await?;
    let online = wait_remote_session_online(state, channel_h, &remote).await?;
    Ok(serde_json::json!({
        "pty_id": "",
        "npub": crate::idref::npub(&remote.pubkey),
        "agent": remote.slug,
        "online_agent": online,
        "channel": channel_h,
        "orchestration_event_id": event_id,
    }))
}

async fn ensure_live_session_member(
    state: &Arc<DaemonState>,
    channel_h: &str,
    rec: &crate::state::Session,
) -> Result<()> {
    refresh_channel_members_cache(state, channel_h).await;
    let is_member = state.with_store(|s| {
        s.is_channel_member(channel_h, &rec.agent_pubkey)
            .unwrap_or(false)
    });
    if is_member {
        return Ok(());
    }

    let added = state
        .provider
        .grant_member_confirmed(channel_h, &rec.agent_pubkey)
        .await;
    if !added.is_confirmed() {
        anyhow::bail!(
            "session {} is not a member of channel {:?} and could not be confirmed as added \
             (is the management key an admin of that channel?)",
            rec.session_id,
            channel_h
        );
    }
    refresh_channel_members_cache(state, channel_h).await;
    Ok(())
}

fn live_pty_for_session(state: &Arc<DaemonState>, rec: &crate::state::Session) -> Option<String> {
    let pty_id = state
        .with_store(|s| s.aliases_for_session(&rec.session_id))
        .ok()?
        .into_iter()
        .find(|a| a.external_id_kind == "pty_session")?
        .external_id;
    if crate::pty::is_live(&pty_id) {
        return Some(pty_id);
    }
    state.with_store(|s| {
        let _ = s.clear_alias_kind(&rec.session_id, "pty_session");
        let _ = s.clear_alias_kind(&rec.session_id, "pty_socket");
    });
    None
}
