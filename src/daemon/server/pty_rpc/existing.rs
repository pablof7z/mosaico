use super::*;

#[derive(serde::Deserialize)]
struct ExistingLaunchParams {
    session: String,
}

pub(in crate::daemon::server) async fn rpc_pty_launch_existing(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ExistingLaunchParams =
        serde_json::from_value(params.clone()).context("parsing pty_launch_existing params")?;
    let Some(rec) = state
        .with_store(|store| super::super::resolution::resolve_public_session(store, &p.session))?
    else {
        return Ok(serde_json::json!({ "action": "not-found" }));
    };
    let handle = state
        .with_store(|store| store.handle_for_pubkey(&rec.pubkey))?
        .unwrap_or_else(|| rec.agent_slug.clone());

    if let Some(pty_id) = live_pty_for_session(state, &rec).await {
        return Ok(serde_json::json!({
            "action": "attached",
            "pty_id": pty_id,
            "handle": handle,
        }));
    }

    let Some(resume_id) = super::resume_token_for(state, &rec) else {
        return Ok(serde_json::json!({
            "action": "not-resumable",
            "handle": handle,
        }));
    };
    let root = state.with_store(|store| super::work_root_for(store, &rec.channel_h));
    let pty_id = crate::session_host::resume_agent_in_channel(
        state,
        &rec.agent_slug,
        &root,
        &rec.channel_h,
        &resume_id,
    )
    .await?;
    Ok(serde_json::json!({
        "action": "resumed",
        "pty_id": pty_id,
        "handle": handle,
    }))
}

async fn live_pty_for_session(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
) -> Option<String> {
    if !rec.alive {
        return None;
    }
    let pty_id = super::pty_session_for_pubkey(state, &rec.pubkey)?;
    for _ in 0..40 {
        if crate::pty::is_live(&pty_id) {
            return Some(pty_id);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    None
}
