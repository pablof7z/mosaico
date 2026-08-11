use super::resolution::work_root_for;
use super::*;

mod message;
mod resolve;
mod session;
mod wait;
use channel_membership_rpc::durable_agent;
use resolve::RemoteSession;
use session::invite_session;
use wait::{
    channel_member_pubkeys, live_session_ids, wait_local_agent_online, wait_local_session_online,
    wait_remote_agent_online,
};

#[derive(serde::Deserialize)]
struct InviteParams {
    channel: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    harness_session: Option<String>,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    pty_session: Option<String>,
    #[serde(default)]
    watch_pid: Option<i32>,
    #[serde(default)]
    cwd: Option<String>,
    /// `channel add --message`: an optional chat line to post into the channel,
    /// mentioning the brought-online session, once it is confirmed online.
    #[serde(default)]
    add_message: Option<String>,
}

pub(super) async fn rpc_invite(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: InviteParams = serde_json::from_value(params.clone()).context("invite params")?;
    let session = p.session.as_deref().filter(|s| !s.trim().is_empty());
    let Some(session_id) = session else {
        anyhow::bail!("invite requires a session; use dispatch to start a new agent session");
    };

    let channel_h = resolve_target_channel(state, &p)?;
    let work_root = state.with_store(|s| work_root_for(s, &channel_h))?;
    let mut result = invite_session(state, &channel_h, &work_root, session_id).await?;
    if let Some(obj) = result.as_object_mut() {
        obj.insert("channel".into(), serde_json::json!(p.channel));
    }
    maybe_post_add_message(state, params, &channel_h, &p, &mut result).await;
    Ok(result)
}

/// `channel add --message`: post a courtesy chat once the target is online.
async fn maybe_post_add_message(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    channel_h: &str,
    p: &InviteParams,
    result: &mut serde_json::Value,
) {
    let Some(message) = p.add_message.as_deref().filter(|m| !m.trim().is_empty()) else {
        return;
    };
    let label = result["online_agent"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if let Some(err) = message::post_add_message(state, params, channel_h, &label, message).await {
        if let Some(obj) = result.as_object_mut() {
            obj.insert("message_error".into(), serde_json::json!(err));
        }
    }
}

fn resolve_target_channel(state: &Arc<DaemonState>, p: &InviteParams) -> Result<String> {
    let anchor = CallerAnchor {
        pty_session: p.pty_session.as_deref(),
        harness_session: p.harness_session.as_deref(),
        watch_pid: p.watch_pid,
        harness: p.harness.as_deref(),
        ..Default::default()
    };
    // Resolution below is global (not scoped to a caller workspace), but
    // invite still requires a resolvable caller context.
    if resolve_session_inner(state, &anchor, ResolveScope::Strict).is_err() {
        let cwd = p
            .cwd
            .as_deref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        crate::daemon::workspace_path::channel_for_path(&cwd)
            .context("invite must run inside an agent session or channel directory")?;
    }
    absolute::require_full_path("channel", &p.channel)?;
    match state.with_store(|s| absolute::resolve_absolute_channel_ref(s, &p.channel)) {
        ChannelResolution::Unique(h) => Ok(h),
        ChannelResolution::NotFound => {
            anyhow::bail!(
                "{}",
                state.with_store(|s| absolute::describe_missing_channel(s, &p.channel))
            )
        }
    }
}

pub(super) async fn invite_agent(
    state: &Arc<DaemonState>,
    channel_h: &str,
    work_root: &str,
    spec: &str,
    cwd: Option<&str>,
) -> Result<serde_json::Value> {
    let channel_ref =
        state.with_store(|store| crate::channel_ref::full_channel_ref(store, channel_h));
    if channel_ref.is_empty() {
        anyhow::bail!("channel metadata is incomplete; refresh channel state and try again");
    }
    let target = crate::idref::parse_agent_backend_ref(spec)
        .with_context(|| format!("malformed agent {spec:?}: expected agent[@backend-label]"))?;
    if target
        .backend
        .as_deref()
        .is_some_and(|backend| backend != state.host())
    {
        let backend = target.backend.as_deref().unwrap();
        let backend_pubkey = resolve_backend_pubkey(state, backend).await?;
        ensure_backend_admin(state, channel_h, &backend_pubkey).await?;
        let before = channel_member_pubkeys(state, channel_h);
        let event_id = publish_invite_orchestration(
            state,
            channel_h,
            crate::fabric::nip29::orchestration::AddTarget {
                backend_pubkey: backend_pubkey.clone(),
                slug: target.slug.clone(),
                session_pubkey: None,
            },
        )
        .await?;
        let online =
            wait_remote_agent_online(state, channel_h, &target.slug, backend, &before).await?;
        return Ok(serde_json::json!({
            "agent": target.slug,
            "online_agent": online,
            "channel": channel_ref,
            "pty_id": "",
            "orchestration_event_id": event_id,
        }));
    }

    // A durable (`perSessionKey: false`) agent has one fixed pubkey; if it's
    // already running (in this channel or another), admit that live session
    // into `channel_h` instead of racing it with a second launch attempt.
    if let Some(rec) = durable_agent::running_durable_session(state, &target.slug) {
        durable_agent::admit_running_agent(state, &rec, channel_h).await?;
        let online = wait_local_session_online(state, channel_h, &rec.pubkey).await?;
        return Ok(serde_json::json!({
            "pty_id": "",
            "agent": target.slug,
            "online_agent": online,
            "channel": channel_ref,
            "host": state.host(),
        }));
    }

    let before = live_session_ids(state);
    super::pty_rpc::provision_before_spawn(state, &target.slug, work_root, Some(channel_h)).await?;
    let spawn = crate::session_host::spawn_agent(
        state,
        &target.slug,
        work_root,
        crate::session_host::SpawnRequest {
            group: Some(channel_h),
            client_cwd: cwd.map(std::path::Path::new),
            session_name: None,
            extra_args: &[],
            intent: crate::session_host::LaunchIntent::Managed,
        },
    )
    .await?;
    let online = wait_local_agent_online(state, channel_h, &target.slug, &before).await?;
    Ok(serde_json::json!({
        "pty_id": spawn.endpoint.endpoint_id,
        "agent": target.slug,
        "online_agent": online,
        "channel": channel_ref,
        "host": state.host(),
    }))
}

/// Cap on the (otherwise unbounded) channel-readiness probe run on the invite
/// RPC's synchronous path. Without it, an unreachable relay wedges the whole
/// invite call — and the client connection with it — indefinitely. Modeled on
/// `rpc/channel.rs`'s `CHANNEL_MEMBER_READY_TIMEOUT`.
const BACKEND_ADMIN_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

pub(super) async fn ensure_backend_admin(
    state: &Arc<DaemonState>,
    channel_h: &str,
    backend_pubkey: &str,
) -> Result<()> {
    let channel = state.with_store(|store| crate::channel_ref::full_channel_ref(store, channel_h));
    if channel.is_empty() {
        anyhow::bail!("channel metadata is incomplete; refresh channel state and try again");
    }
    let mgmt = state.management_keys()?;
    let mgmt_hex = mgmt.public_key().to_hex();
    let parent = state
        .with_store(|s| s.channel_parent(channel_h).unwrap_or(None))
        .filter(|p| !p.is_empty());
    let provider = state.provider();
    let ready = provider.ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
        channel: channel_h,
        expect_member: &mgmt_hex,
        parent_hint: parent.as_deref(),
        name: None,
    });
    let gate = tokio::time::timeout(BACKEND_ADMIN_READY_TIMEOUT, ready)
        .await
        .with_context(|| {
            format!(
                "channel {channel} readiness timed out after {}s",
                BACKEND_ADMIN_READY_TIMEOUT.as_secs()
            )
        })?;
    gate.require_ready(format!("preparing channel {channel} for remote invite"))?;
    let published = provider
        .grant_admin_published(channel_h, backend_pubkey)
        .await;
    published.require_published(format!(
        "granting backend {} access to channel {}",
        crate::util::pubkey_short(backend_pubkey),
        channel
    ))
}

async fn publish_invite_orchestration(
    state: &Arc<DaemonState>,
    channel_h: &str,
    target: crate::fabric::nip29::orchestration::AddTarget,
) -> Result<String> {
    let keys = state.management_keys()?;
    let prose = if target.session_pubkey.is_some() {
        format!("resume {} in this channel", target.slug)
    } else {
        format!("add {} to this channel", target.slug)
    };
    let builder = crate::fabric::nip29::orchestration::build_add_agents_event(
        channel_h,
        channel_h,
        std::slice::from_ref(&target),
        &prose,
    )?;
    // The directive reaches this backend's own orchestration listener through
    // the group subscription NMP injects the accepted row into (#1182), the
    // same path a peer's directive takes.
    Ok(state
        .nmp()
        .publish_group(channel_h, builder, &keys)?
        .to_hex())
}
