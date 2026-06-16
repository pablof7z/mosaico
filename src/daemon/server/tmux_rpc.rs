use super::resolve_session;
use super::*;

// ── tmux_status ───────────────────────────────────────────────────────────────

pub(super) fn rpc_tmux_status(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    let statuses = crate::tmux::list_endpoint_statuses(state);
    let arr: Vec<serde_json::Value> = statuses
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "session_id": s.session_id,
                "pane_id": s.pane_id,
                "pane_command": s.pane_command,
                "alive": s.alive,
                "registered_at": s.registered_at,
                "last_verified": s.last_verified,
            })
        })
        .collect();
    Ok(serde_json::json!({ "endpoints": arr }))
}

// ── tmux_send (manual pending-message injection) ──────────────────────────────

#[derive(serde::Deserialize)]
struct TmuxSendParams {
    session: String,
}

pub(super) async fn rpc_tmux_send(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TmuxSendParams =
        serde_json::from_value(params.clone()).context("parsing tmux_send params")?;

    // Resolve the session (supports prefix matching via resolve_session fallback).
    let rec = resolve_session(state, Some(&p.session), None, None, None)
        .with_context(|| format!("no session matching {:?}", p.session))?;

    let ep = state
        .with_store(|s| s.get_session_endpoint(&rec.session_id, "tmux"))
        .context("store error")?;

    let ep = match ep {
        Some(e) => e,
        None => {
            return Ok(serde_json::json!({
                "injected": false,
                "reason": "no tmux endpoint registered for this session"
            }));
        }
    };

    let pane_id = ep.target.clone();
    if crate::tmux::pane_alive_pub(&pane_id).is_none() {
        state.with_store(|s| s.delete_session_endpoint(&rec.session_id, "tmux").ok());
        return Ok(serde_json::json!({
            "injected": false,
            "reason": format!("pane {pane_id} is gone; endpoint removed")
        }));
    }

    let injected = crate::tmux::inject_pending_messages_pub(state, &rec, &pane_id).await?;
    state.with_store(|s| {
        s.touch_session_endpoint_verified(&rec.session_id, "tmux", crate::util::now_secs())
            .ok()
    });

    if injected {
        Ok(serde_json::json!({ "injected": true, "pane_id": pane_id }))
    } else {
        Ok(serde_json::json!({
            "injected": false,
            "pane_id": pane_id,
            "reason": "no unread messages for this session"
        }))
    }
}

// ── tmux_spawn ────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TmuxSpawnParams {
    agent: String,
    project: String,
}

pub(super) async fn rpc_tmux_spawn(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TmuxSpawnParams =
        serde_json::from_value(params.clone()).context("parsing tmux_spawn params")?;
    let pane_id = crate::tmux::spawn_agent(state, &p.agent, &p.project).await?;
    Ok(serde_json::json!({ "pane_id": pane_id, "agent": p.agent, "project": p.project }))
}

// ── tmux_attach ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TmuxAttachParams {
    session: String,
}

pub(super) fn rpc_tmux_attach(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TmuxAttachParams =
        serde_json::from_value(params.clone()).context("parsing tmux_attach params")?;
    let rec = resolve_session(state, Some(&p.session), None, None, None)
        .with_context(|| format!("no session matching {:?}", p.session))?;
    let ep = state
        .with_store(|s| s.get_session_endpoint(&rec.session_id, "tmux"))
        .context("store error")?;
    match ep {
        Some(e) => Ok(serde_json::json!({ "pane_id": e.target, "session_id": rec.session_id })),
        None => Ok(serde_json::json!({
            "error": "no tmux endpoint registered for this session"
        })),
    }
}

// ── tmux_resume ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TmuxResumeParams {
    session: String,
}

/// The harness-native resume token for a session, or `None` if we can't resume it.
///
/// Priority: an explicitly-stored `resume_id` (opencode forwards its `ses_*`),
/// else the `session_id` itself — for claude/codex we ADOPT their native id as
/// the session id, so it IS the resume token. Only our own synthetic `te-*` ids
/// (generated when a host supplies none, e.g. opencode without a captured id)
/// are not resume tokens, so those fall through to `None`.
fn resume_token_for(state: &Arc<DaemonState>, rec: &crate::state::SessionRecord) -> Option<String> {
    if let Some(id) = state
        .with_store(|s| s.get_session_resume_id(&rec.session_id))
        .ok()
        .flatten()
    {
        return Some(id);
    }
    if rec.session_id.starts_with("te-") {
        return None;
    }
    Some(rec.session_id.clone())
}

/// Resume a (typically dead) session by replaying its harness with the captured
/// native resume token. Spawns a new tmux window and returns its pane id.
pub(super) async fn rpc_tmux_resume(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TmuxResumeParams =
        serde_json::from_value(params.clone()).context("parsing tmux_resume params")?;

    // Resolve including dead sessions: exact id (get_session) first, then a
    // session-id prefix — resolve_session only matches alive rows by cwd/agent.
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

    // Only resume sessions owned by THIS machine — a remote session's harness
    // lives on another host and can't be replayed locally.
    if rec.host != state.host {
        return Ok(serde_json::json!({
            "error": format!("session lives on host {:?}, not resumable from here", rec.host)
        }));
    }

    let resume_id = match resume_token_for(state, &rec) {
        Some(id) => id,
        None => {
            return Ok(serde_json::json!({
                "error": "session has no resume token (not resumable)"
            }));
        }
    };

    match crate::tmux::resume_agent(state, &rec.agent_slug, &rec.project, &resume_id).await {
        Ok(pane_id) => Ok(serde_json::json!({
            "pane_id": pane_id,
            "session_id": rec.session_id,
            "agent": rec.agent_slug,
        })),
        Err(e) => Ok(serde_json::json!({ "error": format!("{e:#}") })),
    }
}

// ── tmux_resumable ────────────────────────────────────────────────────────────

/// List recent local sessions that are resumable but NOT in a live tmux pane.
/// "Dead" rows only — sessions still alive on the fabric appear in the live list
/// and are resumable from there via `[r]`; this section is the longer tail of
/// sessions that have exited entirely. Newest first.
pub(super) fn rpc_tmux_resumable(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    const LIMIT: usize = 60;
    let host = state.host.clone();
    let candidates =
        state.with_store(|s| s.list_resumable_sessions(&host, LIMIT).unwrap_or_default());

    let arr: Vec<serde_json::Value> = candidates
        .into_iter()
        .filter_map(|(rec, _resume_id)| {
            // Must have a usable resume token (claude/codex: the session id;
            // opencode: a captured ses_*; our synthetic te-* ids: not resumable).
            resume_token_for(state, &rec)?;
            // Alive sessions are shown in the live list (resume them with [r]
            // there); keep this section to fully-exited ones to avoid dupes.
            if rec.alive {
                return None;
            }
            // Skip sessions with a live pane — those are attachable, not resume
            // candidates. A missing/dead endpoint means the harness is gone.
            let ep = state
                .with_store(|s| s.get_session_endpoint(&rec.session_id, "tmux"))
                .ok()
                .flatten();
            let live_pane = ep
                .as_ref()
                .is_some_and(|e| crate::tmux::pane_alive_pub(&e.target).is_some());
            if live_pane {
                return None;
            }
            let title = state
                .with_store(|s| s.local_session_snapshot(&rec.session_id).ok().flatten())
                .map(|snap| snap.title)
                .unwrap_or_default();
            Some(serde_json::json!({
                "session_id": rec.session_id,
                "slug": rec.agent_slug,
                "project": rec.project,
                "rel_cwd": rec.rel_cwd,
                "alive": rec.alive,
                "created_at": rec.created_at,
                "title": title,
            }))
        })
        .collect();

    Ok(serde_json::json!({ "resumable": arr }))
}
