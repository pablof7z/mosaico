use super::admission;
use crate::daemon::server::DaemonState;
use crate::harness::ResumeMechanism;
use crate::session_host::transport::{select_transport, LaunchSpec, ResumeSpec, TransportImpl};
use anyhow::{Context, Result};
use std::sync::Arc;

mod spawn;
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
    pty_launch: Option<PtyLaunchSpec>,
) -> Result<crate::pty::LaunchMetadata> {
    match transport {
        TransportImpl::Pty(_) => {
            let pty_launch = pty_launch.unwrap_or_default();
            let meta = crate::pty::spawn_session(crate::pty::SpawnSessionArgs {
                id: pty_launch.id,
                agent: slug.to_string(),
                root: root.to_string(),
                cwd: std::path::PathBuf::from(abs_path),
                channel: group.filter(|g| !g.is_empty()).map(str::to_string),
                session_name: session_name.map(str::to_string),
                ephemeral,
                command: command.to_vec(),
                env: pty_launch
                    .env
                    .into_iter()
                    .chain([(String::from("MOSAICO_PUBKEY"), pubkey.to_string())])
                    .collect(),
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
                bundle: crate::identity::agent_launch_config(&crate::config::mosaico_home(), slug)?
                    .harness,
                profile: crate::identity::agent_launch_config(
                    &crate::config::mosaico_home(),
                    slug,
                )?
                .profile,
                root: root.to_string(),
                abs_path: abs_path.to_string(),
                group: group.map(str::to_string),
                ephemeral,
                base_command: command.to_vec(),
                pubkey: pubkey.to_string(),
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

    let source = resolve_configured_source(slug)?;
    let transport = source.transport;
    let abs_path = workspace_abs_path(state, root, None)?;
    let base = source.command;
    let harness = source.harness;
    let reservation =
        admission::reserve_resume(state, slug, harness.as_str(), root, group, resume_id)?;
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
                root: root.to_string(),
                abs_path: abs_path.clone(),
                group: Some(group.to_string()),
                ephemeral: false,
                base_command: base,
                pubkey: reservation.pubkey.clone(),
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

#[derive(Default)]
pub(super) struct PtyLaunchSpec {
    id: Option<String>,
    env: Vec<(String, String)>,
    env_remove: Vec<String>,
}

pub(super) struct ResolvedSource {
    pub(super) transport: TransportImpl,
    pub(super) command: Vec<String>,
    pub(super) harness: crate::session::Harness,
    pub(super) resume: ResumeMechanism,
    pub(super) bundle: String,
    pub(super) profile: Option<String>,
    pub(super) pty_launch: Option<PtyLaunchSpec>,
}

pub(super) fn resolve_configured_source(slug: &str) -> Result<ResolvedSource> {
    let launch = crate::identity::agent_launch_config(&crate::config::mosaico_home(), slug)?;
    let id = crate::pty::new_endpoint_id(slug);
    let scratch = crate::config::mosaico_home()
        .join("harness-profiles")
        .join(&id);
    let resolved = crate::harness::resolve(&launch.harness, launch.profile.as_deref(), &scratch)
        .with_context(|| {
            format!(
                "resolving harness bundle {:?} for agent {slug:?}",
                launch.harness
            )
        })?;
    let transport = select_transport(&launch.harness)?;
    let pty_launch = if matches!(transport, TransportImpl::Pty(_)) {
        resolved.profile.materialize()?;
        let mut env = resolved.profile.extra_env.clone();
        let mut env_remove = Vec::new();
        for directive in resolved.driver.base_env {
            match directive {
                crate::harness::EnvDirective::Set(key, value) => {
                    env.push((key.to_string(), value.to_string()));
                }
                crate::harness::EnvDirective::Remove(key) => {
                    env_remove.push(key.to_string());
                }
            }
        }
        Some(PtyLaunchSpec {
            id: Some(id),
            env,
            env_remove,
        })
    } else {
        None
    };
    Ok(ResolvedSource {
        transport,
        command: resolved.base_argv,
        harness: resolved.harness,
        resume: resolved.driver.resume,
        bundle: launch.harness,
        profile: launch.profile,
        pty_launch,
    })
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
