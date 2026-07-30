use super::{
    admission, kill_endpoint, source::resolve_harness_source, workspace_abs_path, LaunchIntent,
};
use crate::daemon::server::DaemonState;
use crate::harness::ResumeMechanism;
use crate::session_host::transport::{LaunchSpec, ResumeSpec};
use anyhow::{Context, Result};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(crate) struct ResumeRequest<'a> {
    intent: LaunchIntent,
    extra_args: &'a [String],
}

impl<'a> ResumeRequest<'a> {
    pub(crate) fn with_args(intent: LaunchIntent, extra_args: &'a [String]) -> Self {
        Self { intent, extra_args }
    }

    pub(crate) fn without_args(intent: LaunchIntent) -> Self {
        Self {
            intent,
            extra_args: &[],
        }
    }
}

/// Resume one exact persisted Mosaico session with its harness-native token.
pub(crate) async fn resume_agent(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    resume_id: &str,
    request: ResumeRequest<'_>,
) -> Result<String> {
    anyhow::ensure!(
        !resume_id.is_empty(),
        "session has no resume token (not resumable)"
    );
    let group = state
        .with_store(|store| store.list_session_routes(&rec.pubkey))?
        .into_iter()
        .map(|(channel, _)| channel)
        .next()
        .unwrap_or_default();
    resume_session_record(state, rec, &rec.work_root, &group, resume_id, request).await
}

/// Resume an exact persisted identity into a caller-selected channel.
pub(crate) async fn resume_agent_in_channel(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    root: &str,
    group: &str,
    resume_id: &str,
    request: ResumeRequest<'_>,
) -> Result<String> {
    anyhow::ensure!(
        !resume_id.is_empty(),
        "session has no resume token (not resumable)"
    );
    resume_session_record(state, rec, root, group, resume_id, request).await
}

async fn resume_session_record(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    root: &str,
    group: &str,
    resume_id: &str,
    request: ResumeRequest<'_>,
) -> Result<String> {
    let mut channels = state
        .with_store(|store| store.list_session_routes(&rec.pubkey))?
        .into_iter()
        .map(|(channel, _)| channel)
        .collect::<Vec<_>>();
    if !group.is_empty() && !channels.iter().any(|channel| channel == group) {
        channels.push(group.to_string());
    }
    channels.sort();
    channels.dedup();
    let harness = crate::session::Harness::from_str(&rec.observed_harness);
    anyhow::ensure!(
        harness != crate::session::Harness::Unknown,
        "session {} has unknown harness {:?}",
        rec.pubkey,
        rec.observed_harness
    );
    let abs_path = workspace_abs_path(state, root, None)?;
    let source = resolve_harness_source(
        harness,
        &rec.agent_slug,
        Some(&rec.admitted_bundle),
        request.intent,
    )?;
    let identity = resume_identity(state, rec, &source.bundle)?;
    let reservation = admission::reserve_resume_exact(
        state,
        &identity,
        &rec.pubkey,
        &rec.agent_slug,
        harness.as_str(),
        &source.bundle,
        source.transport.kind().as_str(),
        root,
        group,
    )?;
    launch_resume(
        state,
        source,
        reservation,
        &rec.agent_slug,
        root,
        group,
        &channels,
        &abs_path,
        resume_id,
        request.extra_args,
    )
    .await
}

pub(crate) struct AdoptedNativeSession {
    pub(crate) pty_id: String,
    pub(crate) pubkey: String,
}

pub(crate) async fn adopt_native_session(
    state: &Arc<DaemonState>,
    harness: crate::session::Harness,
    cwd: &std::path::Path,
    root: &str,
    resume_id: &str,
    request: ResumeRequest<'_>,
) -> Result<AdoptedNativeSession> {
    let slug = harness.agent_slug();
    let abs_path = workspace_abs_path(state, root, Some(cwd))?;
    let source = resolve_harness_source(harness, slug, None, request.intent)?;
    let reservation = admission::reserve_fresh(
        state,
        &source.identity,
        harness.as_str(),
        &source.bundle,
        source.transport.kind().as_str(),
        root,
        Some(root),
        None,
    )?;
    let owner = state.with_store(|store| {
        store.claim_native_resume_locator(
            &reservation.pubkey,
            harness.as_str(),
            resume_id,
            crate::util::now_secs(),
        )
    })?;
    if owner != reservation.pubkey {
        admission::release(state, &reservation);
        anyhow::bail!(
            "native session {resume_id:?} was adopted concurrently by pubkey {owner}; retry"
        );
    }
    let pubkey = reservation.pubkey.clone();
    let channels = vec![root.to_string()];
    let pty_id = launch_resume(
        state,
        source,
        reservation,
        slug,
        root,
        root,
        &channels,
        &abs_path,
        resume_id,
        request.extra_args,
    )
    .await?;
    Ok(AdoptedNativeSession { pty_id, pubkey })
}

fn resume_identity(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    bundle: &str,
) -> Result<crate::identity::AgentIdentity> {
    if state.with_store(|store| store.is_derived_session_pubkey(&rec.pubkey))? {
        return Ok(crate::identity::AgentIdentity::per_session(
            &rec.agent_slug,
            bundle,
        ));
    }
    crate::identity::load(&crate::config::mosaico_home(), &rec.agent_slug)
}

#[allow(clippy::too_many_arguments)]
async fn launch_resume(
    state: &Arc<DaemonState>,
    mut source: super::source::ResolvedSource,
    reservation: admission::Reservation,
    slug: &str,
    root: &str,
    group: &str,
    channels: &[String],
    abs_path: &str,
    resume_id: &str,
    extra_args: &[String],
) -> Result<String> {
    let transport = source.transport;
    let harness = source.harness;
    let bundle = source.bundle;
    let resume_command =
        build_driver_resume_command(&source.command, source.resume, resume_id, slug, extra_args)?;
    if let Some(rpc) = source.prepared_launch.rpc.as_mut() {
        rpc.argv.extend_from_slice(extra_args);
    }
    let spec = LaunchSpec {
        slug: slug.to_string(),
        native_agent: source.native_agent,
        root: root.to_string(),
        abs_path: abs_path.to_string(),
        group: Some(group.to_string()),
        ephemeral: false,
        session_name: None,
        base_command: resume_command,
        pubkey: reservation.pubkey.clone(),
        agent_nsec: reservation.agent_nsec.clone(),
        prepared: source.prepared_launch,
    };
    let resume = ResumeSpec {
        native_id: resume_id.to_string(),
    };
    let endpoint = match transport.resume(&spec, &resume).await {
        Ok(endpoint) => endpoint,
        Err(error) => {
            admission::release(state, &reservation);
            return Err(error);
        }
    };
    if let Err(error) = crate::daemon::server::session_start::bootstrap_hosted_session_start(
        state,
        &endpoint,
        crate::daemon::server::session_start::bootstrap::HostedSessionStart {
            pubkey: &reservation.pubkey,
            reclaimed_pubkey: None,
            channel: Some(group),
            channels,
            resume_id: Some(resume_id),
            dispatch_event: None,
            session_name: None,
            observed_harness: harness,
            admitted_bundle: &bundle,
            admitted_transport: transport.kind(),
        },
    )
    .await
    {
        kill_endpoint(&transport, &endpoint.endpoint_id).await;
        admission::release(state, &reservation);
        return Err(error.context("registering resumed hosted session"));
    }
    Ok(endpoint.endpoint_id)
}

fn build_driver_resume_command(
    base: &[String],
    mechanism: ResumeMechanism,
    resume_id: &str,
    slug: &str,
    extra_args: &[String],
) -> Result<Vec<String>> {
    let mut command = match mechanism {
        ResumeMechanism::AppendFlag(flag) => {
            let mut command = base.to_vec();
            command.extend([flag.to_string(), resume_id.to_string()]);
            command
        }
        ResumeMechanism::AppendFlags(flags) => {
            let mut command = base.to_vec();
            command.extend(flags.iter().map(|flag| (*flag).to_string()));
            command.push(resume_id.to_string());
            command
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
            command
        }
        ResumeMechanism::AcpSessionLoad
        | ResumeMechanism::AppServerThreadResume
        | ResumeMechanism::None => base.to_vec(),
    };
    command.extend_from_slice(extra_args);
    Ok(command)
}

#[cfg(test)]
#[path = "resume/tests.rs"]
mod tests;
