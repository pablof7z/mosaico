//! Relay-backed channel readiness checks for session start.
//!
//! This module owns the decision to proceed or roll back when the target NIP-29
//! channel cannot be verified. The parent `session_start` module remains the
//! orchestration layer.

use super::super::*;
use anyhow::Result;
use std::sync::Arc;

const START_CHANNEL_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

pub(super) async fn verify_start_channel_ready(
    state: &Arc<DaemonState>,
    channel: &str,
    work_root: &str,
    room_parent: Option<&str>,
    name: Option<&str>,
    agent_pubkey: &str,
) -> Result<()> {
    start_channel_ready(
        state,
        channel,
        work_root,
        room_parent,
        name,
        agent_pubkey,
        None,
    )
    .await
}

async fn start_channel_ready(
    state: &Arc<DaemonState>,
    channel: &str,
    work_root: &str,
    room_parent: Option<&str>,
    name: Option<&str>,
    agent_pubkey: &str,
    progress: Option<&InitProgress>,
) -> Result<()> {
    if let Some(parent) = room_parent {
        ensure_session_room_ready(state, channel, parent, agent_pubkey, progress).await
    } else {
        ensure_existing_channel_ready(state, channel, work_root, name, agent_pubkey).await
    }
}

async fn ensure_session_room_ready(
    state: &Arc<DaemonState>,
    channel: &str,
    parent: &str,
    agent_pubkey: &str,
    progress: Option<&InitProgress>,
) -> Result<()> {
    // Human-initiated session: mint its per-session room under the work-root,
    // then await the relay's kind:39000 echo before opening gates.
    if let Some(prog) = progress {
        prog.emit("nip29", format!("minting per-session room {channel}"));
    }
    let provisioned = matches!(
        tokio::time::timeout(
            START_CHANNEL_READY_TIMEOUT,
            ensure_session_room(state, channel, channel, parent, agent_pubkey),
        )
        .await,
        Ok(true)
    );
    if !provisioned {
        anyhow::bail!(
            "per-session room {channel} (parent {parent}) was not provisioned on the relay; \
             channel readiness remains pending"
        );
    }
    Ok(())
}

async fn ensure_existing_channel_ready(
    state: &Arc<DaemonState>,
    channel: &str,
    work_root: &str,
    name: Option<&str>,
    agent_pubkey: &str,
) -> Result<()> {
    // Channel / orchestration sessions must verify relay-backed channel state.
    let parent_hint = if channel != work_root && !work_root.is_empty() {
        Some(work_root.to_string())
    } else {
        None
    };
    let open = async {
        let ctx = crate::fabric::nip29::readiness::ChannelCtx {
            channel,
            expect_member: agent_pubkey,
            parent_hint: parent_hint.as_deref(),
            name,
            repair_whitelisted_admins: true,
        };
        state.provider.ensure_channel_ready(ctx).await
    };

    match tokio::time::timeout(START_CHANNEL_READY_TIMEOUT, open).await {
        Ok(crate::fabric::nip29::readiness::ChannelGate::Degraded) => {
            anyhow::bail!(
                "channel {channel} was not verified ready on the relay; \
                 channel readiness remains pending"
            );
        }
        Ok(_) => Ok(()),
        Err(_) => {
            anyhow::bail!(
                "ensure_channel_ready timed out for channel {channel}; \
                 channel readiness remains pending"
            );
        }
    }
}
