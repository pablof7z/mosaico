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
    state.schedule_backend_profile_refresh();
}
