use super::*;

pub struct DispatchedSpawn {
    pub pty_id: String,
    pub pubkey: String,
    pub transport: crate::session_host::transport::TransportKind,
}

pub(crate) struct HostedSpawn {
    pub(crate) meta: crate::pty::LaunchMetadata,
    pub(crate) pubkey: String,
}

pub(crate) struct SpawnRequest<'a> {
    pub(crate) group: Option<&'a str>,
    pub(crate) client_cwd: Option<&'a std::path::Path>,
    pub(crate) session_name: Option<&'a str>,
    pub(crate) intent: LaunchIntent,
}

/// Spawn and register a hosted harness in `root`'s directory. Returns the
/// transport metadata the caller needs to attach or report the endpoint.
pub(crate) async fn spawn_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    request: SpawnRequest<'_>,
) -> Result<HostedSpawn> {
    let (meta, pubkey) = spawn_agent_inner(
        state,
        slug,
        root,
        request.group,
        request.client_cwd,
        request.session_name,
        false,
        request.intent,
    )
    .await?;
    Ok(HostedSpawn { meta, pubkey })
}

pub async fn spawn_ephemeral_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
) -> Result<DispatchedSpawn> {
    let (meta, pubkey) = spawn_agent_inner_full(
        state,
        slug,
        root,
        group,
        None,
        None,
        client_cwd,
        None,
        true,
        LaunchIntent::Managed,
    )
    .await?;
    let transport = if meta.socket.is_empty() {
        crate::session_host::transport::TransportKind::Acp
    } else {
        crate::session_host::transport::TransportKind::Pty
    };
    Ok(DispatchedSpawn {
        pty_id: meta.id,
        pubkey,
        transport,
    })
}

pub(crate) async fn spawn_ephemeral_agent_for_pubkey(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
    expected_pubkey: &str,
) -> Result<String> {
    Ok(spawn_agent_inner_full(
        state,
        slug,
        root,
        group,
        None,
        None,
        client_cwd,
        None,
        true,
        LaunchIntent::Managed,
        Some(expected_pubkey),
    )
    .await?
    .0
    .id)
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
        LaunchIntent::Managed,
        None,
    )
    .await?;
    let transport = if meta.socket.is_empty() {
        crate::session_host::transport::TransportKind::Acp
    } else {
        crate::session_host::transport::TransportKind::Pty
    };
    Ok(DispatchedSpawn {
        pty_id: meta.id,
        pubkey,
        transport,
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
    intent: LaunchIntent,
) -> Result<(crate::pty::LaunchMetadata, String)> {
    spawn_agent_inner_full(
        state,
        slug,
        root,
        group,
        None,
        None,
        client_cwd,
        session_name,
        ephemeral,
        intent,
        None,
    )
    .await
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
    intent: LaunchIntent,
    expected_pubkey: Option<&str>,
) -> Result<(crate::pty::LaunchMetadata, String)> {
    let abs_path = workspace_abs_path(state, root, client_cwd)?;
    let resolved = resolve_agent_source(state, slug, std::path::Path::new(&abs_path), intent)?;
    let agent_slug = resolved.identity.slug.clone();
    let retired_advertisements = resolved.retired_advertisements.clone();
    let agent_command = resolved.command.clone();
    let harness = resolved.harness;
    let reservation = match expected_pubkey {
        Some(pubkey) => admission::reserve_fresh_for_pubkey(
            state,
            &resolved.identity,
            harness.as_str(),
            &resolved.bundle,
            resolved.transport.kind().as_str(),
            root,
            group,
            pubkey,
        )?,
        None => admission::reserve_fresh(
            state,
            &resolved.identity,
            harness.as_str(),
            &resolved.bundle,
            resolved.transport.kind().as_str(),
            root,
            group,
            session_name,
        )?,
    };
    let meta = match open_agent_session(
        &resolved.transport,
        &agent_slug,
        root,
        &abs_path,
        &agent_command,
        group,
        session_name,
        ephemeral,
        &reservation.pubkey,
        &reservation.agent_nsec,
        &resolved.bundle,
        resolved.profile.as_deref(),
        resolved.native_agent.as_ref(),
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
            observed_harness: harness,
            admitted_bundle: &resolved.bundle,
            admitted_transport: resolved.transport.kind(),
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
    state.schedule_agent_roster_refresh(retired_advertisements);
    Ok((meta, pubkey))
}
