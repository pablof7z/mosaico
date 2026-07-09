use super::*;
use crate::reconcile::session_start::ChannelReadyIntent;
use std::sync::Arc;

pub(super) fn schedule_channel_ready(
    state: Arc<DaemonState>,
    session_id: String,
    check: Option<ChannelReadyIntent>,
) {
    let Some(check) = check else {
        return;
    };
    tokio::spawn(async move {
        match channel_ready::verify_start_channel_ready(
            &state,
            &check.channel_h,
            &check.work_root,
            check.room_parent.as_deref(),
            check.name.as_deref(),
            &check.signer_pubkey,
        )
        .await
        {
            Ok(()) => publish_root_roster_if_needed(&state, &check.channel_h).await,
            Err(e) => {
                tracing::warn!(
                    session = %session_id,
                    channel = %check.channel_h,
                    error = %e,
                    "session_start channel readiness work failed"
                );
            }
        }
    });
}

async fn publish_root_roster_if_needed(state: &Arc<DaemonState>, channel_h: &str) {
    let is_root = state.with_store(|s| s.is_root_channel(channel_h).unwrap_or(false));
    if !is_root {
        return;
    }
    match publish_local_agent_roster(state, None).await {
        Ok(report) => tracing::info!(
            channel = %channel_h,
            published = report.published,
            removed = report.removed,
            failed = report.failed.len(),
            "published backend agent roster for root channel"
        ),
        Err(e) => tracing::warn!(
            channel = %channel_h,
            error = %e,
            "backend agent roster publish failed for root channel"
        ),
    }
}

pub(super) fn schedule_replay_chat(state: Arc<DaemonState>, channel: String) {
    tokio::spawn(async move {
        replay_channel_chat(&state, &channel).await;
    });
}
