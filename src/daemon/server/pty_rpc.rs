use super::*;

#[path = "pty_rpc/existing.rs"]
mod existing;
mod native_resume;
mod send;
mod spawn;
mod status;
#[cfg(test)]
#[path = "pty_rpc/tests.rs"]
mod tests;

pub(super) use existing::rpc_pty_launch_existing;
pub(super) use native_resume::rpc_pty_resume_native;
pub(super) use send::{provision_before_spawn, rpc_pty_send};
pub(super) use spawn::rpc_pty_spawn;

pub(super) async fn rpc_pty_status(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    status::rpc_pty_status(state).await
}

fn pty_session_for(state: &Arc<DaemonState>, session: &crate::state::Session) -> Option<String> {
    state
        .with_store(|store| {
            store.runtime_locator_for_session(
                &session.pubkey,
                session.runtime_generation,
                crate::state::LOCATOR_PTY,
            )
        })
        .ok()?
        .map(|locator| locator.locator_value)
}

// ── pty_send (manual pending-message injection) ───────────────────────────────

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
    let rec = state
        .with_store(|store| super::resolution::resolve_public_session(store, &p.session))?
        .with_context(|| "PTY attach requires an npub, hex pubkey, or current handle")?;
    match pty_session_for(state, &rec) {
        Some(pty) => Ok(serde_json::json!({
            "pty_id": pty,
            "pubkey": rec.pubkey,
            "npub": crate::idref::npub(&rec.pubkey),
            "handle": state.with_store(|store| store.handle_for_pubkey(&rec.pubkey))?,
        })),
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
pub(in crate::daemon::server) fn resume_token_for(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
) -> Option<String> {
    state
        .with_store(|store| store.native_resume_locator(&rec.pubkey, &rec.observed_harness))
        .ok()
        .flatten()
        .map(|locator| locator.locator_value)
}

pub(super) async fn rpc_pty_resume(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: PtyResumeParams =
        serde_json::from_value(params.clone()).context("parsing pty_resume params")?;

    let selector = p.session.trim().trim_start_matches('@');
    let pubkey = crate::idref::normalize_pubkey(selector)
        .or_else(|| {
            state
                .with_store(|s| s.pubkey_for_handle(selector))
                .ok()
                .flatten()
        })
        .with_context(|| "resume requires a full npub/hex pubkey or current handle")?;
    let rec = state
        .with_store(|s| s.get_session(&pubkey))?
        .with_context(|| {
            format!(
                "no local session for {}",
                crate::idref::npub(&pubkey).unwrap_or(pubkey)
            )
        })?;

    if rec.is_running() {
        return Ok(serde_json::json!({
            "error": "session is already running; refusing to start a second harness process"
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

    match crate::session_host::resume_agent(
        state,
        &rec,
        &resume_id,
        crate::session_host::LaunchIntent::Interactive,
    )
    .await
    {
        Ok(pty_id) => Ok(serde_json::json!({
            "pty_id": pty_id,
            "npub": crate::idref::npub(&rec.pubkey),
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
        if resume_token_for(state, &rec).is_none() || rec.is_running() {
            continue;
        }
        let live_pty = pty_session_for(state, &rec)
            .map(|pty| crate::pty::is_live(&pty))
            .unwrap_or(false);
        if live_pty {
            continue;
        }
        let (work_root, channels) = state.with_store(|store| {
            let work_root = crate::channel_ref::full_channel_ref(store, &rec.work_root);
            let channels = store
                .list_session_routes(&rec.pubkey)?
                .into_iter()
                .filter_map(|(channel, _)| {
                    let path = crate::channel_ref::full_channel_ref(store, &channel);
                    (!path.is_empty()).then_some(path)
                })
                .collect::<Vec<_>>();
            Ok::<_, anyhow::Error>((work_root, channels))
        })?;
        let pubkey = rec.pubkey.clone();
        let npub = crate::idref::npub(&pubkey).unwrap_or_default();
        let handle = state.with_store(|s| s.handle_for_pubkey(&pubkey).ok().flatten());
        arr.push(serde_json::json!({
            "pubkey": pubkey,
            "npub": npub,
            "handle": handle,
            "channels": channels,
            "work_root": work_root,
            "rel_cwd": "",
            "runtime_state": rec.runtime_state.as_str(),
            "created_at": rec.created_at,
            "title": rec.title,
        }));
    }

    Ok(serde_json::json!({ "resumable": arr }))
}
