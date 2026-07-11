use super::resolution::work_root_for;
use super::*;

mod message;
mod resolve;
mod session;
mod wait;
use resolve::RemoteSession;
use session::invite_session;
use wait::{
    channel_member_pubkeys, live_session_ids, wait_local_agent_online, wait_remote_agent_online,
};

#[derive(serde::Deserialize)]
struct InviteParams {
    channel: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    agent_slug: Option<String>,
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

impl InviteParams {
    fn caller_agent(&self) -> Option<&str> {
        self.agent
            .as_deref()
            .or(self.agent_slug.as_deref())
            .filter(|s| !s.trim().is_empty())
    }
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

    let channel_h = match resolve_target_channel(state, &p)? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };
    let work_root = state.with_store(|s| work_root_for(s, &channel_h));
    let mut result = invite_session(state, &channel_h, &work_root, session_id).await?;
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

enum TargetChannel {
    Unique(String),
    Ambiguous(serde_json::Value),
}

fn resolve_target_channel(state: &Arc<DaemonState>, p: &InviteParams) -> Result<TargetChannel> {
    let anchor = CallerAnchor {
        pty_session: p.pty_session.as_deref(),
        harness_session: p.harness_session.as_deref(),
        watch_pid: p.watch_pid,
        harness: p.harness.as_deref(),
        cwd: p.cwd.as_deref(),
        agent: p.caller_agent(),
        ..Default::default()
    };
    let root = match resolve_session_inner(state, &anchor, ResolveScope::Strict) {
        Ok(rec) => state.with_store(|s| root_channel(s, &rec.channel_h)),
        Err(_) => {
            let cwd = p
                .cwd
                .as_deref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            crate::workspace::resolve(&cwd)
                .context("invite must run inside an agent session or channel directory")?
        }
    };
    match state.with_store(|s| resolve_channel_ref(s, &root, &p.channel)) {
        ChannelResolution::Unique(h) => Ok(TargetChannel::Unique(h)),
        ChannelResolution::Ambiguous(refs) => Ok(TargetChannel::Ambiguous(
            serde_json::json!({ "ambiguous": refs, "reference": p.channel }),
        )),
        ChannelResolution::NotFound => {
            anyhow::bail!("no channel matching {:?} in this channel", p.channel)
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
    let target = crate::idref::parse_agent_backend_ref(spec)
        .with_context(|| format!("malformed agent {spec:?}: expected agent[@backend-label]"))?;
    if target
        .backend
        .as_deref()
        .is_some_and(|backend| backend != state.host)
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
            "channel": channel_h,
            "pty_id": "",
            "orchestration_event_id": event_id,
        }));
    }

    let before = live_session_ids(state);
    super::pty_rpc::provision_before_spawn(state, &target.slug, work_root, Some(channel_h)).await?;
    let pty_id = crate::session_host::spawn_agent(
        state,
        &target.slug,
        work_root,
        Vec::new(),
        None,
        Some(channel_h),
        cwd.map(std::path::Path::new),
    )
    .await?;
    let online = wait_local_agent_online(state, channel_h, &target.slug, &before).await?;
    Ok(serde_json::json!({
        "pty_id": pty_id,
        "agent": target.slug,
        "online_agent": online,
        "channel": channel_h,
        "host": state.host,
    }))
}

/// Cap on the (otherwise unbounded) channel-readiness probe run on the invite
/// RPC's synchronous path. Without it, an unreachable relay wedges the whole
/// invite call — and the client connection with it — indefinitely. Modeled on
/// `rpc/channel.rs`'s `CHANNEL_MEMBER_READY_TIMEOUT`.
const BACKEND_ADMIN_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

async fn ensure_backend_admin(
    state: &Arc<DaemonState>,
    channel_h: &str,
    backend_pubkey: &str,
) -> Result<()> {
    let mgmt = state.management_keys()?;
    let mgmt_hex = mgmt.public_key().to_hex();
    let parent = state
        .with_store(|s| s.channel_parent(channel_h).unwrap_or(None))
        .filter(|p| !p.is_empty());
    let ready = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: channel_h,
            expect_member: &mgmt_hex,
            parent_hint: parent.as_deref(),
            name: None,
            repair_whitelisted_admins: true,
        });
    let gate = match tokio::time::timeout(BACKEND_ADMIN_READY_TIMEOUT, ready).await {
        Ok(gate) => gate,
        Err(_) => crate::fabric::nip29::readiness::ChannelGate::Degraded,
    };
    if matches!(gate, crate::fabric::nip29::readiness::ChannelGate::Degraded) {
        anyhow::bail!("channel {channel_h} is not ready for remote invite");
    }
    let confirmed = state
        .provider
        .grant_admin_confirmed(channel_h, backend_pubkey)
        .await;
    if confirmed.is_confirmed() {
        return Ok(());
    }
    anyhow::bail!(
        "backend {} was not confirmed as an admin of channel {channel_h}",
        crate::util::pubkey_short(backend_pubkey)
    )
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
    let signed = state.transport.sign(builder, &keys).await?;
    let event_id = signed.id.to_hex();
    state.transport.publish_event_checked(&signed).await?;
    if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(&signed) {
        handle_orchestration(state, &signed, op).await;
    }
    Ok(event_id)
}
