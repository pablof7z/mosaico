use super::*;

#[derive(serde::Deserialize)]
struct PtySpawnParams {
    agent: String,
    root: String,
    /// The client's cwd, forwarded so the daemon spawns the agent in the
    /// directory the user actually invoked `mosaico launch` from.
    #[serde(default)]
    cwd: Option<String>,
    /// The resolved opaque channel id to scope the spawned session into.
    #[serde(default)]
    channel: Option<String>,
    /// Operator-selected public handle prefix from `launch --name`.
    #[serde(default)]
    session_name: Option<String>,
    /// Optional initial prompt to open the fresh session on. Used by the headless
    /// launch path, where the child lives in the daemon.
    #[serde(default)]
    prompt: Option<String>,
}

pub(in crate::daemon::server) async fn rpc_pty_spawn(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: PtySpawnParams =
        serde_json::from_value(params.clone()).context("parsing pty_spawn params")?;
    let client_cwd = p.cwd.as_deref().map(std::path::Path::new);
    let group = p.channel.as_deref();
    if let Some(name) = p.session_name.as_deref().filter(|name| !name.is_empty()) {
        state.with_store(|s| s.ensure_custom_handle_available(&p.agent, name))?;
    }
    super::provision_before_spawn(state, &p.agent, &p.root, group).await?;
    let meta = crate::session_host::spawn_agent(
        state,
        &p.agent,
        &p.root,
        crate::session_host::SpawnRequest {
            group,
            client_cwd,
            session_name: p.session_name.as_deref(),
        },
    )
    .await?;

    if let Some(prompt) = p.prompt.as_deref().filter(|prompt| !prompt.is_empty()) {
        crate::session_host::deliver_spawn_prompt(&p.agent, &meta.id, prompt).await;
    }
    let endpoint_kind = if meta.socket.is_empty() {
        crate::state::LOCATOR_ACP
    } else {
        crate::state::LOCATOR_PTY
    };
    let handle = state.with_store(|store| {
        let session = store
            .alive_session_for_locator(None, endpoint_kind, &meta.id)?
            .context("spawned endpoint has no registered session")?;
        Ok::<String, anyhow::Error>(
            store
                .handle_for_pubkey(&session.pubkey)?
                .unwrap_or(session.agent_slug),
        )
    })?;
    Ok(serde_json::json!({
        "pty_id": meta.id,
        "pty_socket": meta.socket,
        "handle": handle,
        "agent": p.agent,
        "root": p.root,
    }))
}
