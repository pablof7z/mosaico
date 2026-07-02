//! Relay-backed channel readiness checks for session start.
//!
//! This module owns the decision to proceed or roll back when the target NIP-29
//! channel cannot be verified. The parent `session_start` module remains the
//! orchestration layer.

use super::super::*;
use super::abort_session_start;
use anyhow::Result;
use std::sync::Arc;

pub(super) async fn ensure_start_channel_ready(
    state: &Arc<DaemonState>,
    project: &str,
    work_root: &str,
    room_parent: Option<&str>,
    agent_pubkey: &str,
    session_id: &str,
    progress: Option<&InitProgress>,
) -> Result<()> {
    if let Some(parent) = room_parent {
        ensure_session_room_ready(state, project, parent, agent_pubkey, session_id, progress).await
    } else {
        ensure_existing_channel_ready(state, project, work_root, agent_pubkey, session_id).await
    }
}

async fn ensure_session_room_ready(
    state: &Arc<DaemonState>,
    project: &str,
    parent: &str,
    agent_pubkey: &str,
    session_id: &str,
    progress: Option<&InitProgress>,
) -> Result<()> {
    // Human-initiated session: mint its per-session room under the work-root,
    // then await the relay's kind:39000 echo before opening gates.
    if let Some(prog) = progress {
        prog.emit("nip29", format!("minting per-session room {project}"));
    }
    let provisioned = matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(8),
            ensure_session_room(state, project, project, parent, agent_pubkey),
        )
        .await,
        Ok(true)
    );
    if !provisioned {
        abort_session_start(state, session_id);
        anyhow::bail!(
            "per-session room {project} (parent {parent}) was not provisioned on the relay; \
             refusing to start the session"
        );
    }
    Ok(())
}

async fn ensure_existing_channel_ready(
    state: &Arc<DaemonState>,
    project: &str,
    work_root: &str,
    agent_pubkey: &str,
    session_id: &str,
) -> Result<()> {
    // Project / orchestration sessions must verify relay-backed channel state.
    let parent_hint = if project != work_root && !work_root.is_empty() {
        Some(work_root.to_string())
    } else {
        None
    };
    let open = async {
        let ctx = crate::fabric::nip29::readiness::ChannelCtx {
            channel: project,
            expect_member: agent_pubkey,
            parent_hint: parent_hint.as_deref(),
            name: None,
            repair_whitelisted_admins: false,
        };
        state.provider.ensure_channel_ready(ctx).await
    };

    match tokio::time::timeout(std::time::Duration::from_secs(8), open).await {
        Ok(crate::fabric::nip29::readiness::ChannelGate::Degraded) => {
            abort_session_start(state, session_id);
            anyhow::bail!(
                "channel {project} was not verified ready on the relay; \
                 refusing to start the session"
            );
        }
        Ok(_) => Ok(()),
        Err(_) => {
            abort_session_start(state, session_id);
            anyhow::bail!(
                "ensure_channel_ready timed out for channel {project}; \
                 refusing to start the session"
            );
        }
    }
}
