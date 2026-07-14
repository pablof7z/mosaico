use super::*;

pub(super) fn resolve_existing_pubkey(
    state: &Arc<DaemonState>,
    params: &SessionStartParams,
    harness: &str,
) -> Result<Option<String>> {
    if let Some(pubkey) = params.pubkey.as_deref().filter(|value| !value.is_empty()) {
        return crate::idref::normalize_pubkey(pubkey)
            .map(Some)
            .context("session_start pubkey must be hex or npub");
    }
    let endpoint_kind = if params.endpoint_kind.as_deref() == Some("acp") {
        crate::state::LOCATOR_ACP
    } else {
        crate::state::LOCATOR_PTY
    };
    let endpoint_pubkey = match params
        .pty_session
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        Some(endpoint) => state.with_store(|store| {
            store
                .alive_session_for_locator(None, endpoint_kind, endpoint)
                .map(|session| session.map(|session| session.pubkey))
        })?,
        None => None,
    };
    let lookup = |kind: &str, value: Option<&String>| -> Result<Option<String>> {
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        state.with_store(|store| store.resolve_pubkey_by_locator(harness, kind, value))
    };
    let resolved = endpoint_pubkey
        .or(lookup(
            crate::state::LOCATOR_NATIVE_RESUME,
            params.resume_id.as_ref(),
        )?)
        .or(lookup(
            crate::state::LOCATOR_NATIVE_RESUME,
            params.harness_session.as_ref(),
        )?);
    let has_stronger_locator = params
        .pty_session
        .as_ref()
        .is_some_and(|value| !value.is_empty())
        || params
            .resume_id
            .as_ref()
            .is_some_and(|value| !value.is_empty())
        || params
            .harness_session
            .as_ref()
            .is_some_and(|value| !value.is_empty());
    if resolved.is_some() || has_stronger_locator {
        return Ok(resolved);
    }
    let pid = params.watch_pid.map(|pid| pid.to_string());
    lookup(crate::state::LOCATOR_PID, pid.as_ref())
}

pub(super) fn bind_workspace(
    state: &Arc<DaemonState>,
    cwd: &std::path::Path,
    work_root: &str,
) -> Result<()> {
    if work_root.is_empty() {
        return Ok(());
    }
    let Some(root_path) = crate::workspace::workspace_dir(cwd) else {
        return Ok(());
    };
    state.with_store(|store| {
        store.upsert_workspace(work_root, &root_path.to_string_lossy(), now_secs())
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reserve_generation(
    state: &Arc<DaemonState>,
    params: &SessionStartParams,
    harness: &str,
    pubkey: &str,
    channel: &str,
    now: u64,
    existing: Option<&crate::state::Session>,
) -> Result<u64> {
    if let Some(existing) = existing {
        if existing.agent_slug != params.agent {
            anyhow::bail!(
                "pubkey {pubkey} belongs to agent {:?}, not {:?}",
                existing.agent_slug,
                params.agent
            );
        }
        if existing.alive {
            return Ok(existing.runtime_generation);
        }
    }
    state.with_store(|store| {
        store.reserve_session(&crate::state::RegisterSession {
            pubkey: pubkey.to_string(),
            harness: harness.to_string(),
            agent_slug: params.agent.clone(),
            channel_h: channel.to_string(),
            child_pid: params.watch_pid,
            transcript_path: None,
            now,
        })
    })
}
