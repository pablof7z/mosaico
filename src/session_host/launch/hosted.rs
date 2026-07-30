use super::{admission, source::ResolvedSource};
use crate::daemon::server::DaemonState;
use crate::harness::ResumeMechanism;
use crate::session_host::transport::{EndpointRef, LaunchSpec, ResumeSpec};
use anyhow::{Context, Result};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) enum ConversationOpen<'a> {
    Fresh,
    Resume { native_id: &'a str },
}

impl<'a> ConversationOpen<'a> {
    fn native_id(self) -> Option<&'a str> {
        match self {
            Self::Fresh => None,
            Self::Resume { native_id } => Some(native_id),
        }
    }

    fn registration_context(self) -> &'static str {
        match self {
            Self::Fresh => "registering hosted session",
            Self::Resume { .. } => "registering resumed hosted session",
        }
    }
}

pub(super) struct HostedPlacement<'a> {
    pub(super) root: &'a str,
    pub(super) abs_path: &'a str,
    pub(super) group: Option<&'a str>,
    pub(super) channels: &'a [String],
}

pub(super) struct HostedPresentation<'a> {
    pub(super) ephemeral: bool,
    pub(super) session_name: Option<&'a str>,
    pub(super) dispatch_event: Option<&'a str>,
}

pub(super) struct HostedOpenRequest<'a> {
    pub(super) source: ResolvedSource,
    pub(super) reservation: admission::Reservation,
    pub(super) conversation: ConversationOpen<'a>,
    pub(super) placement: HostedPlacement<'a>,
    pub(super) presentation: HostedPresentation<'a>,
    pub(super) extra_args: &'a [String],
}

pub(super) struct OpenedHostedSession {
    pub(super) endpoint: crate::session_host::transport::SessionEndpoint,
    pub(super) pubkey: String,
}

pub(super) async fn open(
    state: &Arc<DaemonState>,
    request: HostedOpenRequest<'_>,
) -> Result<OpenedHostedSession> {
    let HostedOpenRequest {
        source,
        reservation,
        conversation,
        placement,
        presentation,
        extra_args,
    } = request;
    let ResolvedSource {
        transport,
        command,
        harness,
        resume,
        bundle,
        native_agent,
        identity,
        prepared_launch,
    } = source;
    let (base_command, prepared) = match prepare_commands(
        command,
        prepared_launch,
        resume,
        conversation,
        extra_args,
        &identity.slug,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            admission::release(state, &reservation);
            return Err(error);
        }
    };
    let spec = LaunchSpec {
        slug: identity.slug,
        native_agent,
        root: placement.root.to_string(),
        abs_path: placement.abs_path.to_string(),
        group: placement.group.map(str::to_string),
        ephemeral: presentation.ephemeral,
        session_name: presentation.session_name.map(str::to_string),
        base_command,
        pubkey: reservation.pubkey.clone(),
        agent_nsec: reservation.agent_nsec.clone(),
        prepared,
    };
    let endpoint = match conversation {
        ConversationOpen::Fresh => transport.launch(&spec).await,
        ConversationOpen::Resume { native_id } => {
            transport
                .resume(
                    &spec,
                    &ResumeSpec {
                        native_id: native_id.to_string(),
                    },
                )
                .await
        }
    };
    let endpoint = match endpoint {
        Ok(endpoint) => endpoint,
        Err(error) => {
            admission::release(state, &reservation);
            return Err(error);
        }
    };
    let reclaimed_pubkey = match conversation {
        ConversationOpen::Fresh => reservation.reclaimed_pubkey.as_deref(),
        ConversationOpen::Resume { .. } => None,
    };
    let registered = crate::daemon::server::session_start::bootstrap_hosted_session_start(
        state,
        &endpoint,
        crate::daemon::server::session_start::bootstrap::HostedSessionStart {
            pubkey: &reservation.pubkey,
            reclaimed_pubkey,
            channel: placement.group,
            channels: placement.channels,
            resume_id: conversation.native_id(),
            dispatch_event: presentation.dispatch_event,
            session_name: presentation.session_name,
            observed_harness: harness,
            admitted_bundle: &bundle,
            admitted_transport: transport.kind(),
        },
    )
    .await;
    let pubkey = match registered {
        Ok(pubkey) => pubkey,
        Err(error) => {
            kill_endpoint(&transport, &endpoint.endpoint_id).await;
            admission::release(state, &reservation);
            return Err(error.context(conversation.registration_context()));
        }
    };
    Ok(OpenedHostedSession { endpoint, pubkey })
}

fn prepare_commands(
    mut command: Vec<String>,
    mut prepared: crate::session_host::transport::PreparedLaunch,
    resume: ResumeMechanism,
    conversation: ConversationOpen<'_>,
    extra_args: &[String],
    slug: &str,
) -> Result<(Vec<String>, crate::session_host::transport::PreparedLaunch)> {
    command = match conversation {
        ConversationOpen::Fresh => command,
        ConversationOpen::Resume { native_id } => {
            build_driver_resume_command(&command, resume, native_id, slug)?
        }
    };
    command.extend_from_slice(extra_args);
    if let Some(rpc) = prepared.rpc.as_mut() {
        rpc.argv.extend_from_slice(extra_args);
    }
    Ok((command, prepared))
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
        ResumeMechanism::AppendFlags(flags) => {
            let mut command = base.to_vec();
            command.extend(flags.iter().map(|flag| (*flag).to_string()));
            command.push(resume_id.to_string());
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
        ResumeMechanism::AcpSessionLoad
        | ResumeMechanism::AppServerThreadResume
        | ResumeMechanism::None => Ok(base.to_vec()),
    }
}

async fn kill_endpoint(
    transport: &crate::session_host::transport::TransportImpl,
    endpoint_id: &str,
) {
    let endpoint = EndpointRef {
        kind: transport.kind(),
        endpoint_id: endpoint_id.to_string(),
    };
    let _ = transport.kill(&endpoint).await;
}

#[cfg(test)]
#[path = "hosted/tests.rs"]
mod tests;
