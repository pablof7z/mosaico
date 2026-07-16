use super::admission;
use crate::agent_catalog::NativeAgentActivation;
use crate::daemon::server::DaemonState;
use crate::harness::ResumeMechanism;
use crate::session_host::transport::{LaunchSpec, ResumeSpec, TransportImpl};
use anyhow::{Context, Result};
use std::sync::Arc;

mod source;
mod spawn;
use source::{resolve_agent_source, PtyLaunchSpec};
pub(crate) use spawn::{spawn_agent, SpawnRequest};
pub use spawn::{spawn_dispatched_ephemeral_agent, spawn_ephemeral_agent, DispatchedSpawn};

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
        state
            .refresh_agent_catalog()
            .context("refreshing native agents for recorded workspace")?;
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
    session_name: Option<&str>,
    ephemeral: bool,
    pubkey: &str,
    agent_nsec: &str,
    bundle: &str,
    profile: Option<&str>,
    native_agent: Option<&NativeAgentActivation>,
    pty_launch: Option<PtyLaunchSpec>,
) -> Result<crate::pty::LaunchMetadata> {
    match transport {
        TransportImpl::Pty(_) => {
            let mut pty_launch = pty_launch.unwrap_or_default();
            crate::session_host::agent_env::assign(
                &mut pty_launch.env,
                &mut pty_launch.env_remove,
                pubkey,
                agent_nsec,
            );
            let meta = crate::pty::spawn_session(crate::pty::SpawnSessionArgs {
                id: pty_launch.id,
                agent: slug.to_string(),
                root: root.to_string(),
                cwd: std::path::PathBuf::from(abs_path),
                channel: group.filter(|g| !g.is_empty()).map(str::to_string),
                session_name: session_name.map(str::to_string),
                ephemeral,
                command: command.to_vec(),
                env: pty_launch.env,
                env_remove: pty_launch.env_remove,
            })?;
            Ok(meta)
        }
        TransportImpl::Acp(t) => {
            use crate::session_host::transport::SessionTransport;
            let spec = LaunchSpec {
                slug: slug.to_string(),
                // The bundle NAME (harnesses.json key) is distinct from the agent
                // slug; the ACP transport resolves its harness/driver from this,
                // never from the slug (defect #1).
                bundle: bundle.to_string(),
                profile: profile.map(str::to_string),
                native_agent: native_agent.cloned(),
                root: root.to_string(),
                abs_path: abs_path.to_string(),
                group: group.map(str::to_string),
                ephemeral,
                base_command: command.to_vec(),
                pubkey: pubkey.to_string(),
                agent_nsec: agent_nsec.to_string(),
            };
            let endpoint = t.launch(&spec).await?;
            Ok(endpoint.meta)
        }
    }
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

    let abs_path = workspace_abs_path(state, root, None)?;
    let source = resolve_agent_source(state, slug, std::path::Path::new(&abs_path))?;
    let transport = source.transport;
    let base = source.command;
    let harness = source.harness;
    let reservation = admission::reserve_resume(
        state,
        &source.identity,
        harness.as_str(),
        root,
        group,
        resume_id,
    )?;
    let meta = match &transport {
        TransportImpl::Pty(_) => {
            let resume_command =
                build_driver_resume_command(&base, source.resume, resume_id, slug)?;
            open_agent_session(
                &transport,
                slug,
                root,
                &abs_path,
                &resume_command,
                Some(group),
                None,
                false,
                &reservation.pubkey,
                &reservation.agent_nsec,
                &source.bundle,
                source.profile.as_deref(),
                source.native_agent.as_ref(),
                source.pty_launch,
            )
            .await?
        }
        // ACP/app-server: re-enter the native session by its resume token
        // (`session/load` or `thread/resume`); the driver argv comes from the
        // harness bundle, so no PTY resume-command shaping applies.
        TransportImpl::Acp(t) => {
            use crate::session_host::transport::SessionTransport;
            let spec = LaunchSpec {
                slug: slug.to_string(),
                bundle: source.bundle,
                profile: source.profile,
                native_agent: source.native_agent,
                root: root.to_string(),
                abs_path: abs_path.clone(),
                group: Some(group.to_string()),
                ephemeral: false,
                base_command: base,
                pubkey: reservation.pubkey.clone(),
                agent_nsec: reservation.agent_nsec.clone(),
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
        crate::daemon::server::session_start::bootstrap::PtySessionStart {
            pubkey: &reservation.pubkey,
            reclaimed_pubkey: None,
            channel: Some(group),
            channels: &[],
            resume_id: Some(resume_id),
            dispatch_event: None,
            session_name: None,
        },
    )
    .await
    {
        kill_endpoint(&transport, &pty_id).await;
        admission::release(state, &reservation);
        return Err(e.context("registering resumed hosted session"));
    }
    Ok(pty_id)
}

fn build_driver_resume_command(
    base: &[String],
    mechanism: ResumeMechanism,
    resume_id: &str,
    slug: &str,
) -> Result<Vec<String>> {
    match mechanism {
        ResumeMechanism::AppendFlag(flag) => {
            let mut command = base.to_vec();
            command.extend([flag.to_string(), resume_id.to_string()]);
            Ok(command)
        }
        ResumeMechanism::Subcommand(subcommand) => {
            let (program, args) = base
                .split_first()
                .with_context(|| format!("agent {slug:?} resolved an empty command"))?;
            let mut command = vec![
                program.clone(),
                subcommand.to_string(),
                resume_id.to_string(),
            ];
            command.extend(args.iter().cloned());
            Ok(command)
        }
        _ => anyhow::bail!("agent {slug:?} uses a non-PTY resume mechanism"),
    }
}
