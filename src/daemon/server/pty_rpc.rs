use super::resolution::work_root_for;
use super::resolve_session;
use super::*;

fn pty_session_for_session(state: &Arc<DaemonState>, session_id: &str) -> Option<String> {
    let aliases = state
        .with_store(|s| s.aliases_for_session(session_id))
        .ok()?;
    aliases
        .into_iter()
        .find(|a| a.external_id_kind == "pty_session")
        .map(|a| a.external_id)
}

// ── pty_status ────────────────────────────────────────────────────────────────

pub(super) async fn rpc_pty_status() -> Result<serde_json::Value> {
    let arr: Vec<serde_json::Value> = crate::pty::read_all_metadata()
        .into_iter()
        .map(|meta| {
            let live = crate::pty::is_live(&meta.id);
            serde_json::json!({
                "pty_id": meta.id,
                "socket": meta.socket,
                "agent": meta.agent,
                "project": meta.project,
                "cwd": meta.cwd,
                "command": meta.command,
                "live": live,
            })
        })
        .collect();
    Ok(serde_json::json!({ "endpoints": arr }))
}

// ── pty_send (manual pending-message injection) ───────────────────────────────

#[derive(serde::Deserialize)]
struct PtySendParams {
    session: String,
}

pub(super) async fn rpc_pty_send(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: PtySendParams =
        serde_json::from_value(params.clone()).context("parsing pty_send params")?;

    let rec = resolve_session(
        state,
        &CallerAnchor {
            explicit: Some(&p.session),
            ..Default::default()
        },
    )
    .with_context(|| format!("no session matching {:?}", p.session))?;

    let Some(pty_id) = pty_session_for_session(state, &rec.session_id) else {
        return Ok(serde_json::json!({
            "injected": false,
            "reason": "no PTY endpoint registered for this session"
        }));
    };
    if !crate::pty::is_live(&pty_id) {
        let _ = state.with_store(|s| s.clear_alias_kind(&rec.session_id, "pty_session"));
        let _ = state.with_store(|s| s.clear_alias_kind(&rec.session_id, "pty_socket"));
        return Ok(serde_json::json!({
            "injected": false,
            "pty_id": pty_id,
            "reason": "PTY endpoint is not live"
        }));
    }

    let injected = crate::session_host::inject_pending_messages_pty(state, &rec, &pty_id).await?;
    if injected {
        Ok(serde_json::json!({ "injected": true, "pty_id": pty_id }))
    } else {
        Ok(serde_json::json!({
            "injected": false,
            "pty_id": pty_id,
            "reason": "no unread messages for this session"
        }))
    }
}

// ── pty_spawn ─────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct PtySpawnParams {
    agent: String,
    project: String,
    #[serde(default)]
    command: Vec<String>,
    /// Override the entire base command, replacing what would be resolved from
    /// the agent file. When non-empty, `command` (extra args) is still appended.
    /// Forwarded from `tenex-edge launch -c <string>`.
    #[serde(default)]
    base_command: Vec<String>,
    /// The client's cwd, forwarded so the daemon spawns the agent in the
    /// directory the user actually invoked `tenex-edge launch` from.
    #[serde(default)]
    cwd: Option<String>,
    /// The resolved opaque channel id to scope the spawned session into.
    #[serde(default)]
    channel: Option<String>,
}

pub(super) async fn rpc_pty_spawn(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: PtySpawnParams =
        serde_json::from_value(params.clone()).context("parsing pty_spawn params")?;
    let client_cwd = p.cwd.as_deref().map(std::path::Path::new);
    let base_override = if p.base_command.is_empty() {
        None
    } else {
        Some(p.base_command)
    };
    let group = p.channel.as_deref();

    provision_before_spawn(state, &p.agent, &p.project, group).await?;

    let pty_id = crate::session_host::spawn_agent(
        state,
        &p.agent,
        &p.project,
        p.command,
        base_override,
        group,
        client_cwd,
    )
    .await?;
    Ok(serde_json::json!({ "pty_id": pty_id, "agent": p.agent, "project": p.project }))
}

/// Call `ensure_channel_ready` for the launch scope (the channel if given, else
/// the project root) before the hosted process is opened.
///
/// If the same agent slug already has a live session in the scope, logs a note
/// about the concurrent launch. The actual signer pubkey is selected and
/// admitted by `session_start`; pre-provisioning with the derivation root pubkey
/// would make the first session look like a duplicate to the ordinal allocator.
pub(in crate::daemon::server) async fn provision_before_spawn(
    state: &Arc<DaemonState>,
    slug: &str,
    project: &str,
    channel: Option<&str>,
) -> Result<()> {
    let scope = channel.filter(|g| !g.is_empty()).unwrap_or(project);
    let already_live = state
        .with_store(|s| s.list_alive_sessions())
        .unwrap_or_default()
        .iter()
        .any(|r| r.agent_slug == slug && r.channel_h == scope);
    if already_live {
        tracing::info!(
            slug,
            scope,
            "provision: launching concurrent instance (agent already has live session)"
        );
    }

    let timeout = std::time::Duration::from_secs(20);
    let parent_hint = channel
        .filter(|g| !g.is_empty() && *g != project)
        .map(|_| project);
    let channel_name = state
        .with_store(|s| s.get_channel(scope))
        .ok()
        .flatten()
        .map(|c| c.name)
        .unwrap_or_default();
    tracing::info!(
        slug,
        channel = scope,
        channel_name,
        "provision: ensuring channel ready"
    );
    let expect_member = state.backend_pubkey().unwrap_or_default();
    let ctx = crate::fabric::nip29::readiness::ChannelCtx {
        channel: scope,
        expect_member: &expect_member,
        parent_hint,
        name: None,
        repair_whitelisted_admins: true,
    };
    match tokio::time::timeout(timeout, state.provider.ensure_channel_ready(ctx)).await {
        Ok(crate::fabric::nip29::readiness::ChannelGate::Degraded) => tracing::warn!(
            slug,
            channel = scope,
            "provision: channel readiness degraded before spawn; opening local session anyway"
        ),
        Ok(_) => {}
        Err(_) => tracing::warn!(
            slug,
            channel = scope,
            "provision: channel readiness timed out before spawn; opening local session anyway"
        ),
    }
    Ok(())
}

// ── pty_attach ────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct PtyAttachParams {
    session: String,
}

pub(super) fn rpc_pty_attach(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: PtyAttachParams =
        serde_json::from_value(params.clone()).context("parsing pty_attach params")?;
    let rec = resolve_session(
        state,
        &CallerAnchor {
            explicit: Some(&p.session),
            ..Default::default()
        },
    )
    .with_context(|| format!("no session matching {:?}", p.session))?;
    match pty_session_for_session(state, &rec.session_id) {
        Some(pty) => Ok(serde_json::json!({ "pty_id": pty, "session_id": rec.session_id })),
        None => Ok(serde_json::json!({
            "error": "no PTY endpoint registered for this session"
        })),
    }
}

// ── pty_resume ────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct PtyResumeParams {
    session: String,
}

/// The harness-native resume token for a session, or `None` if we can't resume it.
pub(in crate::daemon::server) fn resume_token_for(rec: &crate::state::Session) -> Option<String> {
    if !rec.resume_id.is_empty() {
        return Some(rec.resume_id.clone());
    }
    if rec.session_id.starts_with("te-") {
        return None;
    }
    Some(rec.session_id.clone())
}

pub(super) async fn rpc_pty_resume(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: PtyResumeParams =
        serde_json::from_value(params.clone()).context("parsing pty_resume params")?;

    let rec = match state
        .with_store(|s| s.get_session(&p.session))
        .ok()
        .flatten()
    {
        Some(r) => r,
        None => state
            .with_store(|s| s.find_session_by_prefix(&p.session))
            .ok()
            .flatten()
            .with_context(|| format!("no session matching {:?}", p.session))?,
    };

    let resume_id = match resume_token_for(&rec) {
        Some(id) => id,
        None => {
            return Ok(serde_json::json!({
                "error": "session has no resume token (not resumable)"
            }));
        }
    };

    let scope = rec.channel_h.clone();
    match crate::session_host::resume_agent(state, &rec.agent_slug, &scope, &resume_id).await {
        Ok(pty_id) => Ok(serde_json::json!({
            "pty_id": pty_id,
            "session_id": rec.session_id,
            "agent": rec.agent_slug,
        })),
        Err(e) => Ok(serde_json::json!({ "error": format!("{e:#}") })),
    }
}

// ── pty_resumable ─────────────────────────────────────────────────────────────

/// List recent local sessions that are resumable but not attached to a live PTY.
pub(super) async fn rpc_pty_resumable(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    const LIMIT: u32 = 60;
    let candidates = state
        .with_store(|s| s.list_resumable_sessions(LIMIT))
        .unwrap_or_default();

    let mut arr = Vec::new();
    for rec in candidates {
        if resume_token_for(&rec).is_none() || rec.alive {
            continue;
        }
        let live_pty = pty_session_for_session(state, &rec.session_id)
            .map(|pty| crate::pty::is_live(&pty))
            .unwrap_or(false);
        if live_pty {
            continue;
        }
        let work_root = state.with_store(|s| work_root_for(s, &rec.channel_h));
        let slug = state.session_instance(&rec).display_slug();
        arr.push(serde_json::json!({
            "session_id": rec.session_id,
            "slug": slug,
            "project": rec.channel_h,
            "work_root": work_root,
            "rel_cwd": "",
            "alive": rec.alive,
            "created_at": rec.created_at,
            "title": rec.title,
        }));
    }

    Ok(serde_json::json!({ "resumable": arr }))
}
