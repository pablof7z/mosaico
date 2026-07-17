//! Relay-backed channel readiness checks for session start.
//!
//! This module owns the decision to proceed or roll back when the target NIP-29
//! channel cannot be verified. The parent `session_start` module remains the
//! orchestration layer.

use super::super::*;
use anyhow::Result;
use std::sync::Arc;

const START_CHANNEL_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

pub(super) fn session_parent_hint(
    state: &Arc<DaemonState>,
    channel: &str,
    work_root: &str,
    room_parent: Option<&str>,
    existing: Option<&crate::state::Session>,
) -> Result<String> {
    let relay_parent = state.with_store(|store| store.channel_parent(channel))?;
    let resolution_parent = state.with_store(|store| store.channel_resolution_parent(channel))?;
    let pending_parent = room_parent
        .or(resolution_parent.as_deref())
        .or_else(|| {
            existing
                .filter(|session| session.channel_h == channel)
                .map(|session| session.readiness_parent.as_str())
                .filter(|parent| !parent.is_empty())
        })
        .or_else(|| (channel != work_root && !work_root.is_empty()).then_some(work_root));
    Ok(crate::fabric::nip29::readiness::effective_parent_hint(
        relay_parent,
        pending_parent,
        channel,
    )
    .unwrap_or_default())
}

pub(super) async fn verify_start_channel_ready(
    state: &Arc<DaemonState>,
    channel: &str,
    room_parent: Option<&str>,
    readiness_parent: Option<&str>,
    name: Option<&str>,
    agent_pubkey: &str,
) -> Result<()> {
    start_channel_ready(
        state,
        channel,
        room_parent,
        readiness_parent,
        name,
        agent_pubkey,
        None,
    )
    .await
}

async fn start_channel_ready(
    state: &Arc<DaemonState>,
    channel: &str,
    room_parent: Option<&str>,
    readiness_parent: Option<&str>,
    name: Option<&str>,
    agent_pubkey: &str,
    progress: Option<&InitProgress>,
) -> Result<()> {
    if let Some(parent) = room_parent {
        ensure_session_room_ready(state, channel, parent, agent_pubkey, progress).await
    } else {
        ensure_existing_channel_ready(state, channel, readiness_parent, name, agent_pubkey).await
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
    readiness_parent: Option<&str>,
    name: Option<&str>,
    agent_pubkey: &str,
) -> Result<()> {
    // Channel / orchestration sessions must verify relay-backed channel state.
    let open = async {
        let ctx = crate::fabric::nip29::readiness::ChannelCtx {
            channel,
            expect_member: agent_pubkey,
            parent_hint: readiness_parent,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pending_nested_channel_keeps_its_immediate_parent() {
        let state = DaemonState::new_for_test().await;
        state
            .with_store(|store| {
                store.reserve_channel_resolution_intent("parent", "leaf", "leaf-h", 1)
            })
            .unwrap();

        assert_eq!(
            session_parent_hint(&state, "leaf-h", "workspace", None, None).unwrap(),
            "parent"
        );

        state
            .with_store(|store| store.upsert_channel("leaf-h", "leaf", "", "", 2))
            .unwrap();
        assert_eq!(
            session_parent_hint(&state, "leaf-h", "workspace", None, None).unwrap(),
            "",
            "relay-authored root metadata must suppress pending local ancestry"
        );

        let old = state
            .with_store(|store| {
                store.reserve_hook_session_for_test(&crate::state::RegisterSession {
                    pubkey: "pk".into(),
                    observed_harness: "codex".into(),
                    agent_slug: "agent".into(),
                    channel_h: "old-room".into(),
                    child_pid: None,
                    transcript_path: None,
                    now: 1,
                })?;
                store.set_session_context("pk", "old-room", "workspace", "old-parent")?;
                store.get_session("pk")
            })
            .unwrap()
            .expect("session");
        assert_eq!(
            session_parent_hint(&state, "new-room", "workspace", None, Some(&old)).unwrap(),
            "workspace",
            "an old channel's pending parent must not leak into a new channel"
        );
    }
}
