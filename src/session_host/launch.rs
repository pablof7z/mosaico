use super::admission;
use crate::daemon::server::DaemonState;
use crate::session_host::registry::{
    apply_agent_def_args, build_resume_command, find_spawn_def, resolve_spawn_entry,
    resume_shape_for_bin,
};
use crate::session_host::transport::{select_transport, LaunchSpec, ResumeSpec, TransportImpl};
use anyhow::{Context, Result};
use std::sync::Arc;

/// Resolve which transport hosts `slug`, from its configured harness bundle. An
/// agent with no bundle (the overwhelming majority) resolves to the PTY, and its
/// launch path is byte-identical to before this wiring existed.
fn transport_for_slug(slug: &str) -> Result<TransportImpl> {
    let bundle = crate::identity::agent_harness_bundle(&crate::config::edge_home(), slug);
    select_transport(bundle.as_deref())
}

/// Kill a just-opened endpoint through its transport (PTY supervisor or ACP
/// child) — used to roll back a session whose registration failed.
async fn kill_endpoint(transport: &TransportImpl, endpoint_id: &str) {
    use crate::session_host::transport::EndpointRef;
    let ep = EndpointRef {
        kind: transport.kind(),
        endpoint_id: endpoint_id.to_string(),
    };
    let _ = transport.kill(&ep).await;
}

/// Resolve the base spawn command + inline agent definition for `slug`.
///
/// PTY agents must have a resolvable harness command (unchanged). An ACP/
/// app-server agent is launched from its harness bundle's driver argv, not a PTY
/// command, so when it has no `commands` entry we synthesize a nominal command
/// (the bundle's harness slug) purely so harness inference + recorded session
/// metadata are correct; the actual child argv comes from the bundle driver.
fn resolve_spawn_command(
    slug: &str,
    transport: &TransportImpl,
) -> Result<(Vec<String>, Option<serde_json::Value>)> {
    match resolve_spawn_entry(slug) {
        Ok(v) => Ok(v),
        Err(e) => {
            if matches!(transport, TransportImpl::Acp(_)) {
                let bundle =
                    crate::identity::agent_harness_bundle(&crate::config::edge_home(), slug);
                let cfg = crate::harness::config::HarnessesConfig::load()?;
                let harness =
                    crate::harness::bundle_harness_with(&cfg, bundle.as_deref().unwrap_or(slug))?;
                Ok((vec![harness.as_str().to_string()], None))
            } else {
                Err(e)
            }
        }
    }
}

pub struct DispatchedSpawn {
    pub pty_id: String,
    pub session_id: String,
}

pub(super) fn workspace_abs_path(
    state: &Arc<DaemonState>,
    channel: &str,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    if let Some(cwd) = client_cwd {
        let abs = cwd.to_string_lossy().to_string();
        let now = crate::util::now_secs();
        // The recorded workspace path is what the resume path reads back; if the
        // write is dropped, a later resume falls into the "no workspace" branch and
        // we'd spawn in the wrong directory. Propagate the failure, don't swallow.
        state
            .with_store(|s| s.upsert_workspace(channel, &abs, now))
            .with_context(|| format!("recording workspace path for {channel:?}"))?;
        return Ok(abs);
    }
    // Resume path (no client cwd): the workspace path MUST already be recorded.
    // Never guess the daemon's current_dir here; an unrelated daemon cwd would
    // land the agent in the wrong directory. Fail loud on a read error or
    // missing row.
    let abs = state
        .with_store(|s| s.workspace_path(channel))
        .with_context(|| format!("looking up workspace path for {channel:?}"))?;
    abs.ok_or_else(|| {
        anyhow::anyhow!("cannot resolve workspace path for {channel:?} (no recorded path)")
    })
}

#[allow(clippy::too_many_arguments)]
async fn open_agent_session(
    transport: &TransportImpl,
    slug: &str,
    root: &str,
    abs_path: &str,
    command: &[String],
    group: Option<&str>,
    ephemeral: bool,
    durable_reservation: Option<String>,
) -> Result<crate::pty::LaunchMetadata> {
    match transport {
        // PTY path: byte-identical to the pre-wiring `open_agent_session` body,
        // including the durable_reservation the supervisor holds until exit.
        TransportImpl::Pty(_) => {
            let meta = crate::pty::spawn_session(crate::pty::SpawnSessionArgs {
                id: None,
                agent: slug.to_string(),
                root: root.to_string(),
                cwd: std::path::PathBuf::from(abs_path),
                channel: group.filter(|g| !g.is_empty()).map(str::to_string),
                ephemeral,
                durable_reservation,
                command: command.to_vec(),
            })?;
            Ok(meta)
        }
        // ACP/app-server path: launch the RPC child and hand back the synthesized
        // LaunchMetadata so session registration is transport-agnostic. The
        // durable reservation is bound at registration (bootstrap carries it) and
        // released via session-liveness + orphan cleanup, so it is not threaded
        // into the child here (there is no supervisor to hold it).
        TransportImpl::Acp(t) => {
            use crate::session_host::transport::SessionTransport;
            let spec = LaunchSpec {
                slug: slug.to_string(),
                root: root.to_string(),
                abs_path: abs_path.to_string(),
                group: group.map(str::to_string),
                ephemeral,
                base_command: command.to_vec(),
            };
            let endpoint = t.launch(&spec).await?;
            Ok(endpoint.meta)
        }
    }
}

/// Spawn a new PTY-hosted harness in `root`'s directory. Returns the
/// supervisor session id.
pub async fn spawn_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    launch_args: Vec<String>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    spawn_agent_inner(
        state,
        slug,
        root,
        launch_args,
        base_override,
        group,
        client_cwd,
        false,
    )
    .await
}

pub async fn spawn_ephemeral_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    launch_args: Vec<String>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    spawn_agent_inner(
        state,
        slug,
        root,
        launch_args,
        base_override,
        group,
        client_cwd,
        true,
    )
    .await
}

pub async fn spawn_dispatched_ephemeral_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    channels: &[String],
    dispatch_event: &str,
) -> Result<DispatchedSpawn> {
    if channels.is_empty() {
        anyhow::bail!("dispatch spawn requires at least one channel");
    }
    let (pty_id, session_id) = spawn_agent_inner_full(
        state,
        slug,
        root,
        Vec::new(),
        None,
        Some(&channels[0]),
        Some(channels),
        Some(dispatch_event),
        None,
        true,
    )
    .await?;
    Ok(DispatchedSpawn { pty_id, session_id })
}

#[allow(clippy::too_many_arguments)]
async fn spawn_agent_inner(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    launch_args: Vec<String>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
    ephemeral: bool,
) -> Result<String> {
    Ok(spawn_agent_inner_full(
        state,
        slug,
        root,
        launch_args,
        base_override,
        group,
        None,
        None,
        client_cwd,
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
    launch_args: Vec<String>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    joined_channels: Option<&[String]>,
    dispatch_event: Option<&str>,
    client_cwd: Option<&std::path::Path>,
    ephemeral: bool,
) -> Result<(String, String)> {
    let transport = transport_for_slug(slug)?;
    let (base_command, agent_def) = match base_override {
        Some(cmd) => (cmd, None),
        None => resolve_spawn_command(slug, &transport)?,
    };
    let mut agent_command = apply_agent_def_args(base_command, slug, agent_def);
    if !launch_args.is_empty() {
        agent_command.extend(launch_args);
    }
    let _ = find_spawn_def(slug);

    let abs_path = workspace_abs_path(state, root, client_cwd)?;
    let reservation = admission::reserve(state, slug)?;
    let opened = open_agent_session(
        &transport,
        slug,
        root,
        &abs_path,
        &agent_command,
        group,
        ephemeral,
        reservation.clone(),
    )
    .await;
    let meta = match opened {
        Ok(meta) => meta,
        Err(error) => {
            admission::release(state, reservation.as_deref());
            return Err(error);
        }
    };
    let pty_id = meta.id.clone();
    let empty_channels: &[String] = &[];
    let channels = joined_channels.unwrap_or(empty_channels);
    let session_id = match crate::daemon::server::session_start::bootstrap_pty_session_start(
        state,
        &meta,
        group,
        channels,
        None,
        dispatch_event,
        reservation.as_deref(),
    )
    .await
    {
        Ok(session_id) => session_id,
        Err(e) => {
            kill_endpoint(&transport, &pty_id).await;
            return Err(e.context("registering hosted session"));
        }
    };
    Ok((pty_id, session_id))
}

/// Resume a prior session by replaying its harness with the native resume token.
pub async fn resume_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    resume_id: &str,
) -> Result<String> {
    resume_agent_in_channel(state, slug, root, root, resume_id).await
}

/// Resume a prior session into an explicit channel while using `root` to
/// resolve the working directory.
pub async fn resume_agent_in_channel(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    group: &str,
    resume_id: &str,
) -> Result<String> {
    if resume_id.is_empty() {
        anyhow::bail!("session has no resume token (not resumable)");
    }

    let transport = transport_for_slug(slug)?;
    let abs_path = workspace_abs_path(state, root, None)?;
    // A resumed claude/codex session re-registers under the SAME session_id, so it
    // deterministically re-derives its own pubkey — no explicit hint needed.
    let meta = match &transport {
        TransportImpl::Pty(_) => {
            let (base, _agent_def) = resolve_spawn_entry(slug)?;
            let bin = base.first().map(String::as_str).unwrap_or("");
            let shape = resume_shape_for_bin(bin).with_context(|| {
                format!("don't know how to resume harness binary {bin:?} (agent {slug:?})")
            })?;
            let resume_command = build_resume_command(&base, shape, resume_id);
            open_agent_session(
                &transport,
                slug,
                root,
                &abs_path,
                &resume_command,
                Some(group),
                false,
                None,
            )
            .await?
        }
        // ACP/app-server: re-enter the native session by its resume token
        // (`session/load` or `thread/resume`); the driver argv comes from the
        // harness bundle, so no PTY resume-command shaping applies.
        TransportImpl::Acp(t) => {
            use crate::session_host::transport::SessionTransport;
            let (base, _agent_def) = resolve_spawn_command(slug, &transport)?;
            let spec = LaunchSpec {
                slug: slug.to_string(),
                root: root.to_string(),
                abs_path: abs_path.clone(),
                group: Some(group.to_string()),
                ephemeral: false,
                base_command: base,
            };
            let resume = ResumeSpec {
                native_id: resume_id.to_string(),
            };
            t.resume(&spec, &resume).await?.meta
        }
    };
    let pty_id = meta.id.clone();
    if let Err(e) = crate::daemon::server::session_start::bootstrap_pty_session_start(
        state,
        &meta,
        Some(group),
        &[],
        Some(resume_id),
        None,
        None,
    )
    .await
    {
        kill_endpoint(&transport, &pty_id).await;
        return Err(e.context("registering resumed hosted session"));
    }
    Ok(pty_id)
}
