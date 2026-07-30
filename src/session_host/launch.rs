use super::admission;
use crate::daemon::server::DaemonState;
use anyhow::{Context, Result};
use std::sync::Arc;

mod hosted;
mod resume;
mod resume_request;
mod source;
mod spawn;
pub(crate) use resume::{adopt_native_session, resume_agent, resume_agent_in_channel};
pub(crate) use resume_request::ResumeRequest;
use source::resolve_agent_source;
pub(crate) use spawn::spawn_ephemeral_agent_for_pubkey;
pub(crate) use spawn::{spawn_agent, SpawnRequest};
pub use spawn::{spawn_dispatched_ephemeral_agent, spawn_ephemeral_agent, DispatchedSpawn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchIntent {
    /// A human invoked a direct launch and needs an attachable PTY.
    Interactive,
    /// Fabric provisioning prefers the harness's hosted RPC transport.
    Managed,
}

pub(super) fn workspace_abs_path(
    state: &Arc<DaemonState>,
    channel: &str,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    if let Some(cwd) = client_cwd {
        let abs = cwd.to_string_lossy().to_string();
        if !channel.is_empty() {
            let now = crate::util::now_secs();
            // The recorded workspace path is what the resume path reads back; if
            // the write is dropped, a later resume falls into the "no workspace"
            // branch and we'd spawn in the wrong directory. Unscoped launches
            // deliberately have no shared root binding.
            state
                .with_store(|s| {
                    crate::daemon::workspace_path::WorkspacePathResolver::new(s)
                        .bind_root_path(channel, cwd, now)
                })
                .with_context(|| format!("recording workspace path for {channel:?}"))?;
        }
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
