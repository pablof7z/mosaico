use super::admission;
use crate::agent_catalog::NativeAgentActivation;
use crate::daemon::server::DaemonState;
use crate::session_host::transport::{LaunchSpec, PreparedLaunch, SessionEndpoint, TransportImpl};
use anyhow::{Context, Result};
use std::sync::Arc;

mod resume;
mod source;
mod spawn;
pub(crate) use resume::{
    adopt_native_session, resume_agent, resume_agent_in_channel, resume_session_record,
};
use source::resolve_agent_source;
pub(crate) use spawn::spawn_ephemeral_agent_for_pubkey;
pub(crate) use spawn::{spawn_agent, SpawnRequest};
pub use spawn::{spawn_dispatched_ephemeral_agent, spawn_ephemeral_agent, DispatchedSpawn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchIntent {
    /// A human invoked `mosaico agents` and needs an attachable PTY.
    Interactive,
    /// Fabric provisioning prefers the harness's hosted RPC transport.
    Managed,
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
            .with_store(|s| {
                crate::daemon::workspace_path::WorkspacePathResolver::new(s)
                    .bind_root_path(channel, cwd, now)
            })
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
        .with_store(|s| {
            crate::daemon::workspace_path::WorkspacePathResolver::new(s).path_for_channel(channel)
        })
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
    native_agent: Option<&NativeAgentActivation>,
    prepared_launch: PreparedLaunch,
) -> Result<SessionEndpoint> {
    let spec = LaunchSpec {
        slug: slug.to_string(),
        native_agent: native_agent.cloned(),
        root: root.to_string(),
        abs_path: abs_path.to_string(),
        group: group.map(str::to_string),
        ephemeral,
        session_name: session_name.map(str::to_string),
        base_command: command.to_vec(),
        pubkey: pubkey.to_string(),
        agent_nsec: agent_nsec.to_string(),
        prepared: prepared_launch,
    };
    transport.launch(&spec).await
}
