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
                .filter(|session| {
                    state.with_store(|store| {
                        store
                            .has_session_route(&session.pubkey, channel)
                            .unwrap_or(false)
                    })
                })
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
    let gate = tokio::time::timeout(
        START_CHANNEL_READY_TIMEOUT,
        ensure_session_room(state, channel, channel, parent, agent_pubkey),
    )
    .await
    .with_context(|| {
        format!(
            "per-session room {channel} readiness timed out after {}s",
            START_CHANNEL_READY_TIMEOUT.as_secs()
        )
    })?;
    gate.require_ready(format!(
        "per-session room {channel} below {parent} was not provisioned"
    ))
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
        };
        state.provider().ensure_channel_ready(ctx).await
    };

    let gate = tokio::time::timeout(START_CHANNEL_READY_TIMEOUT, open)
        .await
        .with_context(|| {
            format!(
                "ensure_channel_ready timed out for channel {channel} after {}s",
                START_CHANNEL_READY_TIMEOUT.as_secs()
            )
        })?;
    gate.require_ready(format!(
        "session start could not verify channel {channel} readiness"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAILURE: &str =
        "fault=latched durability=absent reopen=required: Previous I/O error occurred";

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
                    launch_channel_h: "old-room".into(),
                    work_root: "workspace".into(),
                    child_pid: None,
                    now: 1,
                })?;
                store.set_session_readiness_parent("pk", "old-parent")?;
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

    #[tokio::test]
    async fn session_start_readiness_keeps_exact_checked_publish_failure() {
        let state =
            DaemonState::new_for_test_with_relays(vec!["wss://relay.example.com".into()]).await;
        state.nmp().script_read_events(Vec::new());
        state
            .nmp()
            .script_write_error("scripted NMP publish refusal", FAILURE);
        state.nmp().script_read_events(Vec::new());

        let error = verify_start_channel_ready(
            &state,
            "missing-root",
            None,
            None,
            None,
            &nostr::Keys::generate().public_key().to_hex(),
        )
        .await
        .expect_err("session readiness must fail");
        let rendered = format!("{error:#}");
        assert!(rendered.contains(FAILURE), "{rendered}");
        assert!(
            rendered.contains("9007 create-group NMP publish failed"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("readiness remains pending"),
            "{rendered}"
        );
    }
}
