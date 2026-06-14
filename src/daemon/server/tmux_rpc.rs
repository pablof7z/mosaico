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

// ── tmux_send (manual doorbell) ───────────────────────────────────────────────

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

    crate::tmux::inject_doorbell_pub(&pane_id).await?;
    state.with_store(|s| {
        s.touch_session_endpoint_verified(&rec.session_id, "tmux", crate::util::now_secs())
            .ok()
    });

    Ok(serde_json::json!({ "injected": true, "pane_id": pane_id }))
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
