use crate::daemon::server::DaemonState;
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate};
use crate::state::Session;
use anyhow::Result;
use std::sync::Arc;

pub(super) fn parent_hint(state: &DaemonState, session: &Session, channel: &str) -> Option<String> {
    let relay_parent = state.with_store(|store| store.channel_parent(channel).ok().flatten());
    crate::fabric::nip29::readiness::effective_parent_hint(
        relay_parent,
        Some(&session.readiness_parent),
        channel,
    )
}

pub(super) fn workspace(state: &DaemonState, session: &Session) -> Result<String> {
    state.with_store(|store| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(store).root_for_session(session)
    })
}

/// Re-establish every durable membership independently. Relay-authored parent
/// state wins; the admission-time parent is only a bootstrap hint while
/// metadata is incomplete.
pub(super) async fn restore_routes(state: &Arc<DaemonState>, session: &Session, routes: &[String]) {
    for channel in routes {
        let parent_hint = parent_hint(state, session, channel);
        let _lane = state.standing_sync.lock().await;
        if !super::super::managed_lifecycle::admission_is_current(
            state,
            &session.pubkey,
            channel,
            session.runtime_generation,
            session.lifecycle_epoch,
            false,
        ) {
            tracing::debug!(
                pubkey = %session.pubkey,
                runtime_generation = session.runtime_generation,
                %channel,
                "daemon reconciliation skipped a route that is no longer current"
            );
            continue;
        }
        let gate = state
            .snapshot()
            .provider
            .ensure_channel_ready(ChannelCtx {
                channel,
                expect_member: &session.pubkey,
                parent_hint: parent_hint.as_deref(),
                name: None,
            })
            .await;
        if let ChannelGate::Degraded(error) = gate {
            tracing::warn!(
                pubkey = %session.pubkey,
                agent = %session.agent_slug,
                %channel,
                error = %error,
                "channel not verified ready on reconcile; retaining live session and retrying later"
            );
            continue;
        }
        match super::super::managed_lifecycle::commit_confirmed_admission(
            state,
            &session.pubkey,
            channel,
            session.runtime_generation,
            session.lifecycle_epoch,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                pubkey = %session.pubkey,
                runtime_generation = session.runtime_generation,
                %channel,
                "reconciled membership became stale"
            ),
            Err(error) => tracing::error!(
                pubkey = %session.pubkey,
                %channel,
                %error,
                "reconciled membership admission could not be persisted"
            ),
        }
    }
}
