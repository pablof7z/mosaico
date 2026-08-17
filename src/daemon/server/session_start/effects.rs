use super::advisory::ChannelReadyIntent;
use super::*;
use std::sync::Arc;

pub(super) fn schedule_channel_ready(
    state: Arc<DaemonState>,
    pubkey: String,
    runtime_generation: u64,
    lifecycle_epoch: u64,
    check: Option<ChannelReadyIntent>,
) {
    let Some(check) = check else {
        return;
    };
    tokio::spawn(async move {
        let _lane = state.standing_sync.lock().await;
        let already_confirmed = state
            .with_store(|store| {
                Ok::<_, anyhow::Error>(
                    store.has_session_route(&check.pubkey, &check.channel_h)?
                        && store
                            .get_session_standing(&check.pubkey, &check.channel_h)?
                            .is_some_and(|standing| {
                                standing.state == crate::state::StandingState::Member
                                    && standing.session_lifecycle_epoch == lifecycle_epoch
                            }),
                )
            })
            .unwrap_or(false);
        if already_confirmed {
            return;
        }
        if !super::super::managed_lifecycle::admission_is_current(
            &state,
            &check.pubkey,
            &check.channel_h,
            runtime_generation,
            lifecycle_epoch,
            true,
        ) {
            tracing::debug!(
                pubkey,
                channel = %check.channel_h,
                lifecycle_epoch,
                "session_start channel admission was cancelled by newer membership state"
            );
            return;
        }
        match channel_ready::verify_start_channel_ready(
            &state,
            &check.channel_h,
            check.room_parent.as_deref(),
            check.readiness_parent.as_deref(),
            check.name.as_deref(),
            &check.pubkey,
        )
        .await
        {
            Ok(()) => {
                match super::super::managed_lifecycle::commit_confirmed_admission(
                    &state,
                    &check.pubkey,
                    &check.channel_h,
                    runtime_generation,
                    lifecycle_epoch,
                )
                .await
                {
                    Ok(true) => publish_host_profile_if_root(&state, &check.channel_h).await,
                    Ok(false) => {
                        tracing::warn!(pubkey, channel = %check.channel_h, lifecycle_epoch, "confirmed channel admission became stale")
                    }
                    Err(error) => {
                        tracing::error!(pubkey, channel = %check.channel_h, lifecycle_epoch, %error, "confirmed channel admission persistence failed")
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    pubkey,
                    channel = %check.channel_h,
                    error = %render_channel_ready_failure(&error),
                    "session_start channel readiness work failed"
                );
            }
        }
    });
}

fn render_channel_ready_failure(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

async fn publish_host_profile_if_root(state: &Arc<DaemonState>, channel_h: &str) {
    let is_root = state.with_store(|s| s.is_root_channel(channel_h).unwrap_or(false));
    if !is_root {
        return;
    }
    match publish_backend_profile(state).await {
        Ok(report) => tracing::info!(
            channel = %channel_h,
            agents = report.agents,
            workspaces = report.workspaces,
            failed = report.failed.len(),
            "published backend host profile for root workspace"
        ),
        Err(e) => tracing::warn!(
            channel = %channel_h,
            error = %e,
            "backend host profile publish failed for root workspace"
        ),
    }
}

pub(super) fn schedule_replay_chat(state: Arc<DaemonState>, channel: String) {
    tokio::spawn(async move {
        replay_channel_chat(&state, &channel).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    // Scripted future-classified receipt matching newer NMP behavior. The
    // pinned NMP revision cannot originate this classification itself.
    const SCRIPTED_CLASSIFIED_FAILURE: &str =
        "fault=latched durability=absent reopen=required: Previous I/O error occurred";

    fn absent_group_snapshot(group: &str) -> nmp::nip29::GroupSnapshot {
        nmp::nip29::GroupSnapshot {
            id: group.to_string(),
            metadata: None,
            admins: Vec::new(),
            members: Vec::new(),
            availability: nmp::nip29::GroupAvailability::Ready,
            per_host: BTreeMap::new(),
            disagreements: BTreeSet::new(),
        }
    }

    #[tokio::test]
    async fn nonblocking_readiness_sink_renders_complete_failure_chain_once() {
        let state =
            DaemonState::new_for_test_with_relays(vec!["wss://relay.example.com".into()]).await;
        for _ in 0..4 {
            state
                .snapshot()
                .nmp
                .script_group_snapshot(absent_group_snapshot("missing-root"));
        }
        state
            .snapshot()
            .nmp
            .script_write_error("scripted NMP publish refusal", SCRIPTED_CLASSIFIED_FAILURE);

        let error = channel_ready::verify_start_channel_ready(
            &state,
            "missing-root",
            None,
            None,
            None,
            &nostr::Keys::generate().public_key().to_hex(),
        )
        .await
        .expect_err("background readiness work must fail");
        let rendered = render_channel_ready_failure(&error);
        assert_eq!(
            rendered.matches(SCRIPTED_CLASSIFIED_FAILURE).count(),
            1,
            "{rendered}"
        );
        assert!(
            rendered.contains("9007 create-group NMP publish failed"),
            "{rendered}"
        );
    }
}
