use super::*;

#[derive(serde::Deserialize)]
struct NativeResumeParams {
    native_id: String,
    #[serde(default)]
    workspace: Option<String>,
}

pub(in crate::daemon::server) async fn rpc_pty_resume_native(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: NativeResumeParams =
        serde_json::from_value(params.clone()).context("parsing pty_resume_native params")?;
    let native_id = params.native_id.trim();
    anyhow::ensure!(!native_id.is_empty(), "native session id is empty");

    let locators = state.with_store(|store| {
        store.locators_for_value(None, crate::state::LOCATOR_NATIVE_RESUME, native_id)
    })?;
    if locators.len() > 1 {
        anyhow::bail!(
            "native session id {native_id:?} is mapped by multiple harnesses: {}",
            locators
                .iter()
                .map(|locator| locator.harness.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if let Some(locator) = locators.first() {
        anyhow::ensure!(
            params.workspace.is_none(),
            "--workspace cannot move an already mapped Mosaico session"
        );
        let rec = state
            .with_store(|store| store.get_session(&locator.pubkey))?
            .with_context(|| {
                format!(
                    "native session id {native_id:?} maps to missing pubkey {}",
                    locator.pubkey
                )
            })?;
        return resume_mapped(state, &rec, native_id).await;
    }

    let matches = crate::session_host::discover_native_session(native_id)?;
    anyhow::ensure!(
        !matches.is_empty(),
        "native session id {native_id:?} was not found in any supported harness"
    );
    if matches.len() > 1 {
        anyhow::bail!(
            "native session id {native_id:?} is ambiguous across: {}",
            matches
                .iter()
                .map(|candidate| candidate.harness.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let discovered = &matches[0];
    let cwd = params
        .workspace
        .map(std::path::PathBuf::from)
        .or_else(|| discovered.cwd.clone())
        .with_context(|| {
            format!(
                "{} session {native_id:?} has no recorded cwd; pass --workspace PATH",
                discovered.harness.as_str()
            )
        })?;
    anyhow::ensure!(
        cwd.is_absolute() && cwd.is_dir(),
        "workspace {} is not an existing absolute directory",
        cwd.display()
    );
    let root = crate::daemon::workspace_path::channel_for_path_or_bail(&cwd)?;
    super::provision_before_spawn(state, discovered.harness.agent_slug(), &root, None).await?;
    let adopted = crate::session_host::adopt_native_session(
        state,
        discovered.harness,
        &cwd,
        &root,
        native_id,
    )
    .await?;
    let handle = state
        .with_store(|store| store.handle_for_pubkey(&adopted.pubkey))?
        .unwrap_or_else(|| discovered.harness.agent_slug().to_string());
    Ok(serde_json::json!({
        "action": "adopted",
        "pty_id": adopted.pty_id,
        "handle": handle,
        "agent": discovered.harness.agent_slug(),
        "harness": discovered.harness.as_str(),
        "pubkey": adopted.pubkey,
        "npub": crate::idref::npub(&adopted.pubkey),
    }))
}

async fn resume_mapped(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    native_id: &str,
) -> Result<serde_json::Value> {
    let handle = state
        .with_store(|store| store.handle_for_pubkey(&rec.pubkey))?
        .unwrap_or_else(|| rec.agent_slug.clone());
    if let Some(pty_id) = super::existing::live_pty_for_session(state, rec).await {
        return Ok(response("attached", &pty_id, &handle, rec));
    }
    anyhow::ensure!(
        !rec.is_running(),
        "@{handle} is already running without an attachable PTY; use `mosaico sessions` for explicit takeover"
    );
    let root = state.with_store(|store| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(store).root_for_session(rec)
    })?;
    let pty_id = crate::session_host::resume_session_record(
        state,
        rec,
        &root,
        &rec.channel_h,
        native_id,
        crate::session_host::LaunchIntent::Interactive,
    )
    .await?;
    Ok(response("resumed", &pty_id, &handle, rec))
}

fn response(
    action: &str,
    pty_id: &str,
    handle: &str,
    rec: &crate::state::Session,
) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "pty_id": pty_id,
        "handle": handle,
        "agent": rec.agent_slug,
        "harness": rec.observed_harness,
        "pubkey": rec.pubkey,
        "npub": crate::idref::npub(&rec.pubkey),
    })
}

#[cfg(test)]
#[path = "native_resume/tests.rs"]
mod tests;
