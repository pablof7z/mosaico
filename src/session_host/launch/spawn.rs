use super::*;

pub struct DispatchedSpawn {
    pub pty_id: String,
    pub pubkey: String,
}

pub(crate) struct SpawnRequest<'a> {
    pub(crate) group: Option<&'a str>,
    pub(crate) client_cwd: Option<&'a std::path::Path>,
    pub(crate) session_name: Option<&'a str>,
}

/// Spawn and register a hosted harness in `root`'s directory. Returns the
/// transport metadata the caller needs to attach or report the endpoint.
pub(crate) async fn spawn_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    request: SpawnRequest<'_>,
) -> Result<crate::pty::LaunchMetadata> {
    spawn_agent_inner(
        state,
        slug,
        root,
        request.group,
        request.client_cwd,
        request.session_name,
        false,
    )
    .await
}

pub async fn spawn_ephemeral_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    Ok(
        spawn_agent_inner(state, slug, root, group, client_cwd, None, true)
            .await?
            .id,
    )
}

pub async fn spawn_dispatched_ephemeral_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    channels: &[String],
    dispatch_event: &str,
) -> Result<DispatchedSpawn> {
    let group = channels
        .first()
        .context("dispatch spawn requires at least one channel")?;
    let (meta, pubkey) = spawn_agent_inner_full(
        state,
        slug,
        root,
        Some(group),
        Some(channels),
        Some(dispatch_event),
        None,
        None,
        true,
    )
    .await?;
    Ok(DispatchedSpawn {
        pty_id: meta.id,
        pubkey,
    })
}

#[allow(clippy::too_many_arguments)]
async fn spawn_agent_inner(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
    session_name: Option<&str>,
    ephemeral: bool,
) -> Result<crate::pty::LaunchMetadata> {
    Ok(spawn_agent_inner_full(
        state,
        slug,
        root,
        group,
        None,
        None,
        client_cwd,
        session_name,
        ephemeral,
    )
    .await?
    .0)
}

#[allow(clippy::too_many_arguments)]
async fn spawn_agent_inner_full(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    group: Option<&str>,
    joined_channels: Option<&[String]>,
    dispatch_event: Option<&str>,
    client_cwd: Option<&std::path::Path>,
    session_name: Option<&str>,
    ephemeral: bool,
) -> Result<(crate::pty::LaunchMetadata, String)> {
    let resolved = resolve_configured_source(slug)?;
    let agent_command = resolved.command;

    let abs_path = workspace_abs_path(state, root, client_cwd)?;
    let harness = resolved.harness;
    let reservation =
        admission::reserve_fresh(state, slug, harness.as_str(), root, group, session_name)?;
    let meta = match open_agent_session(
        &resolved.transport,
        slug,
        root,
        &abs_path,
        &agent_command,
        group,
        session_name,
        ephemeral,
        &reservation.pubkey,
        resolved.pty_launch,
    )
    .await
    {
        Ok(meta) => meta,
        Err(error) => {
            admission::release(state, &reservation);
            return Err(error);
        }
    };
    let pty_id = meta.id.clone();
    let channels = joined_channels.unwrap_or(&[]);
    let pubkey = match crate::daemon::server::session_start::bootstrap_pty_session_start(
        state,
        &meta,
        crate::daemon::server::session_start::bootstrap::PtySessionStart {
            pubkey: &reservation.pubkey,
            reclaimed_pubkey: reservation.reclaimed_pubkey.as_deref(),
            channel: group,
            channels,
            resume_id: None,
            dispatch_event,
            session_name,
        },
    )
    .await
    {
        Ok(pubkey) => pubkey,
        Err(e) => {
            kill_endpoint(&resolved.transport, &pty_id).await;
            admission::release(state, &reservation);
            return Err(e.context("registering hosted session"));
        }
    };
    Ok((meta, pubkey))
}
