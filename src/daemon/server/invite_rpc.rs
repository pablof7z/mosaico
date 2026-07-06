use super::resolution::work_root_for;
use super::*;

mod wait;
use wait::{
    channel_member_pubkeys, live_session_ids, wait_local_agent_online, wait_local_session_online,
    wait_remote_agent_online, wait_remote_session_online,
};

#[derive(serde::Deserialize)]
struct InviteParams {
    channel: String,
    #[serde(default)]
    target_agent: Option<String>,
    #[serde(default)]
    invite_agent: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    agent_slug: Option<String>,
    #[serde(default, alias = "env_session")]
    harness_session: Option<String>,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    tmux_pane: Option<String>,
    #[serde(default)]
    watch_pid: Option<i32>,
    #[serde(default)]
    cwd: Option<String>,
}

impl InviteParams {
    fn invitee(&self) -> Option<&str> {
        self.target_agent
            .as_deref()
            .or(self.invite_agent.as_deref())
            .or_else(|| self.agent_slug.as_ref().and(self.agent.as_deref()))
            .filter(|s| !s.trim().is_empty())
    }

    fn caller_agent(&self) -> Option<&str> {
        self.agent_slug
            .as_deref()
            .or_else(|| {
                (self.target_agent.is_some()
                    || self.invite_agent.is_some()
                    || self.session.is_some())
                .then_some(())
                .and(self.agent.as_deref())
            })
            .filter(|s| !s.trim().is_empty())
    }
}

pub(super) async fn rpc_invite(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: InviteParams = serde_json::from_value(params.clone()).context("invite params")?;
    let agent = p.invitee();
    let session = p.session.as_deref().filter(|s| !s.trim().is_empty());
    match (agent.is_some(), session.is_some()) {
        (true, true) => anyhow::bail!("invite requires exactly one of agent or session"),
        (false, false) => anyhow::bail!("invite requires exactly one of agent or session"),
        _ => {}
    }

    let channel_h = match resolve_target_channel(state, &p)? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };
    let work_root = state.with_store(|s| work_root_for(s, &channel_h));
    if let Some(session_id) = session {
        return invite_session(state, &channel_h, &work_root, session_id).await;
    }
    invite_agent(
        state,
        &channel_h,
        &work_root,
        agent.unwrap(),
        p.cwd.as_deref(),
    )
    .await
}

enum TargetChannel {
    Unique(String),
    Ambiguous(serde_json::Value),
}

fn resolve_target_channel(state: &Arc<DaemonState>, p: &InviteParams) -> Result<TargetChannel> {
    let anchor = CallerAnchor {
        tmux_pane: p.tmux_pane.as_deref(),
        harness_session: p.harness_session.as_deref(),
        watch_pid: p.watch_pid,
        harness: p.harness.as_deref(),
        cwd: p.cwd.as_deref(),
        agent: p.caller_agent(),
        ..Default::default()
    };
    let root = match resolve_session_inner(state, &anchor, ResolveScope::Strict) {
        Ok(rec) => state.with_store(|s| project_root(s, &rec.channel_h)),
        Err(_) => {
            let cwd = p
                .cwd
                .as_deref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            crate::project::resolve(&cwd)
                .context("invite must run inside an agent session or project directory")?
        }
    };
    match state.with_store(|s| resolve_channel_ref(s, &root, &p.channel)) {
        ChannelResolution::Unique(h) => Ok(TargetChannel::Unique(h)),
        ChannelResolution::Ambiguous(refs) => Ok(TargetChannel::Ambiguous(
            serde_json::json!({ "ambiguous": refs, "reference": p.channel }),
        )),
        ChannelResolution::NotFound => {
            anyhow::bail!("no channel matching {:?} in this project", p.channel)
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
                session_id: None,
            },
        )
        .await?;
        let online =
            wait_remote_agent_online(state, channel_h, &target.slug, backend, &before).await?;
        return Ok(serde_json::json!({
            "agent": target.slug,
            "online_agent": online,
            "channel": channel_h,
            "pane_id": "",
            "orchestration_event_id": event_id,
        }));
    }

    let before = live_session_ids(state);
    super::tmux_rpc::provision_before_spawn(state, &target.slug, work_root, Some(channel_h))
        .await?;
    let pane_id = crate::tmux::spawn_agent(
        state,
        &target.slug,
        work_root,
        Vec::new(),
        None,
        Some(channel_h),
        cwd.map(std::path::Path::new),
        None,
    )
    .await?;
    let online = wait_local_agent_online(state, channel_h, &target.slug, &before).await?;
    Ok(serde_json::json!({
        "pane_id": pane_id,
        "agent": target.slug,
        "online_agent": online,
        "channel": channel_h,
    }))
}

async fn invite_session(
    state: &Arc<DaemonState>,
    channel_h: &str,
    work_root: &str,
    session_id: &str,
) -> Result<serde_json::Value> {
    if let Some(rec) = local_session(state, session_id) {
        let resume_id = super::tmux_rpc::resume_token_for(&rec).with_context(|| {
            format!(
                "session {} has no resume token (not resumable)",
                rec.session_id
            )
        })?;
        super::tmux_rpc::provision_before_spawn(state, &rec.agent_slug, work_root, Some(channel_h))
            .await?;
        let pane_id = crate::tmux::resume_agent_in_channel(
            state,
            &rec.agent_slug,
            work_root,
            channel_h,
            &resume_id,
        )
        .await?;
        let online = wait_local_session_online(state, channel_h, &rec.session_id).await?;
        return Ok(serde_json::json!({
            "pane_id": pane_id,
            "session_id": rec.session_id,
            "agent": rec.agent_slug,
            "online_agent": online,
            "channel": channel_h,
        }));
    }

    let remote = remote_session_from_status(state, session_id)?;
    if remote.backend == state.host {
        anyhow::bail!(
            "session {} appears to belong to this backend, but no local session row exists",
            remote.session_id
        );
    }
    let backend_pubkey = resolve_backend_pubkey(state, &remote.backend).await?;
    ensure_backend_admin(state, channel_h, &backend_pubkey).await?;
    let event_id = publish_invite_orchestration(
        state,
        channel_h,
        crate::fabric::nip29::orchestration::AddTarget {
            backend_pubkey,
            slug: remote.slug.clone(),
            session_id: Some(remote.session_id.clone()),
        },
    )
    .await?;
    let online = wait_remote_session_online(state, channel_h, &remote).await?;
    Ok(serde_json::json!({
        "pane_id": "",
        "session_id": remote.session_id,
        "agent": remote.slug,
        "online_agent": online,
        "channel": channel_h,
        "orchestration_event_id": event_id,
    }))
}

fn local_session(state: &Arc<DaemonState>, session: &str) -> Option<crate::state::Session> {
    state
        .with_store(|s| s.get_session(session))
        .ok()
        .flatten()
        .or_else(|| {
            state
                .with_store(|s| s.find_session_by_prefix(session))
                .ok()
                .flatten()
        })
}

struct RemoteSession {
    session_id: String,
    pubkey: String,
    slug: String,
    backend: String,
}

fn remote_session_from_status(state: &Arc<DaemonState>, session: &str) -> Result<RemoteSession> {
    let matches = state.with_store(|s| -> Result<Vec<RemoteSession>> {
        let mut out = Vec::new();
        for st in s.list_status_sessions(None, None)? {
            if st.session_id != session && !st.session_id.starts_with(session) {
                continue;
            }
            let Some(profile) = s.get_profile(&st.pubkey)? else {
                continue;
            };
            let slug = if profile.slug.is_empty() {
                st.slug.clone()
            } else {
                profile.slug.clone()
            };
            out.push(RemoteSession {
                session_id: st.session_id,
                pubkey: st.pubkey,
                slug,
                backend: profile.host,
            });
        }
        out.sort_by(|a, b| {
            a.session_id
                .cmp(&b.session_id)
                .then(a.pubkey.cmp(&b.pubkey))
        });
        out.dedup_by(|a, b| a.session_id == b.session_id && a.pubkey == b.pubkey);
        Ok(out)
    })?;
    match matches.as_slice() {
        [one] => Ok(RemoteSession {
            session_id: one.session_id.clone(),
            pubkey: one.pubkey.clone(),
            slug: one.slug.clone(),
            backend: one.backend.clone(),
        }),
        [] => anyhow::bail!("no session matching {session:?}"),
        _ => anyhow::bail!("session id {session:?} is ambiguous; use the full session id"),
    }
}

async fn ensure_backend_admin(
    state: &Arc<DaemonState>,
    channel_h: &str,
    backend_pubkey: &str,
) -> Result<()> {
    let mgmt = state.management_keys()?;
    let parent = state
        .with_store(|s| s.channel_parent(channel_h).unwrap_or(None))
        .filter(|p| !p.is_empty());
    let gate = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: channel_h,
            expect_member: &mgmt.public_key().to_hex(),
            parent_hint: parent.as_deref(),
            name: None,
            repair_whitelisted_admins: true,
        })
        .await;
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
    let prose = if target.session_id.is_some() {
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
