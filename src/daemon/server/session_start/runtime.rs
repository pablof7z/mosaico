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
    let endpoint_pubkey = match params.hosted_endpoint()? {
        Some((endpoint, kind)) => state.with_store(|store| {
            store
                .alive_session_for_locator(harness, kind.locator_kind(), endpoint)
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
    let Some(root_path) = crate::daemon::workspace_path::root_path_for(cwd)? else {
        return Ok(());
    };
    state.with_store(|store| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(store).bind_root_path(
            work_root,
            &root_path,
            now_secs(),
        )
    })
}

pub(super) fn bind_locators(
    store: &crate::state::Store,
    params: &SessionStartParams,
    harness: &str,
    pubkey: &str,
    now: u64,
) -> Result<()> {
    if let Some((endpoint, kind)) = params.hosted_endpoint()? {
        store.put_session_locator(harness, kind.locator_kind(), endpoint, pubkey, now)?;
    }
    if let Some(native) = params
        .resume_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            params
                .harness_session
                .as_deref()
                .filter(|value| !value.is_empty())
        })
    {
        store.set_native_resume_locator(pubkey, harness, native, now)?;
    }
    if let Some(pid) = params.watch_pid {
        store.put_session_locator(
            harness,
            crate::state::LOCATOR_PID,
            &pid.to_string(),
            pubkey,
            now,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reserve_generation(
    state: &Arc<DaemonState>,
    params: &SessionStartParams,
    facts: &super::params::RuntimeFacts,
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
        store.reserve_session_with_facts(
            &crate::state::RegisterSession {
                pubkey: pubkey.to_string(),
                observed_harness: facts.observed_harness.as_str().to_string(),
                agent_slug: params.agent.clone(),
                channel_h: channel.to_string(),
                child_pid: params.watch_pid,
                transcript_path: None,
                now,
            },
            &crate::state::AdmittedRuntimeFacts {
                observed_harness: facts.observed_harness.as_str().to_string(),
                claimed_harness: facts.claimed_harness.clone(),
                bundle: facts.admitted_bundle.clone(),
                transport: facts.admitted_transport.clone(),
                endpoint_provenance: facts.endpoint_provenance.clone(),
            },
        )
    })
}

#[cfg(test)]
#[path = "runtime/tests.rs"]
mod tests;
