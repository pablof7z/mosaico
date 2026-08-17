use super::*;

pub(super) async fn compensate_due_admission(
    state: &Arc<DaemonState>,
    due: &crate::state::SessionStanding,
) {
    let removal = state
        .snapshot()
        .provider
        .remove_member_published(&due.channel_h, &due.pubkey)
        .await;
    if !removal.is_published() {
        tracing::warn!(
            pubkey = %pubkey_short(&due.pubkey),
            channel = %due.channel_h,
            ?removal,
            "admission compensation remains durably due"
        );
        return;
    }
    match state.with_store(|store| {
        store.mark_member_standing_absent_if_epoch(
            &due.pubkey,
            &due.channel_h,
            due.standing_epoch,
            due.session_lifecycle_epoch,
            now_secs(),
        )
    }) {
        Ok(true) => tracing::info!(
            pubkey = %pubkey_short(&due.pubkey),
            channel = %due.channel_h,
            "stale confirmed admission was removed"
        ),
        Ok(false) => tracing::debug!(
            pubkey = %pubkey_short(&due.pubkey),
            channel = %due.channel_h,
            "admission compensation was superseded while removal completed"
        ),
        Err(error) => tracing::error!(
            pubkey = %pubkey_short(&due.pubkey),
            channel = %due.channel_h,
            %error,
            "confirmed admission removal could not be persisted; cleanup remains due"
        ),
    }
}

pub(super) async fn reconcile_running(state: &Arc<DaemonState>) {
    let due = state
        .with_store(|store| store.list_cleanup_due_member_standing())
        .unwrap_or_else(|error| {
            tracing::error!(%error, "standing cleanup scan failed");
            Vec::new()
        });
    for standing in due {
        let _lane = state.standing_sync.lock().await;
        let still_due = state
            .with_store(|store| {
                let current = store.get_session_standing(&standing.pubkey, &standing.channel_h)?;
                let routed = store.has_session_route(&standing.pubkey, &standing.channel_h)?;
                Ok::<_, anyhow::Error>(current.as_ref() == Some(&standing) && !routed)
            })
            .unwrap_or_else(|error| {
                tracing::error!(
                    pubkey = %pubkey_short(&standing.pubkey),
                    channel = %standing.channel_h,
                    %error,
                    "standing cleanup revalidation failed"
                );
                false
            });
        if still_due {
            compensate_due_admission(state, &standing).await;
        }
    }

    let sessions = state.with_store(|store| store.list_running_sessions().unwrap_or_default());
    for session in sessions {
        let routes = state
            .with_store(|store| store.list_session_routes(&session.pubkey))
            .unwrap_or_default();
        for (channel, _) in routes {
            let member = state
                .with_store(|store| store.get_session_standing(&session.pubkey, &channel))
                .ok()
                .flatten()
                .is_some_and(|standing| standing.state == crate::state::StandingState::Member);
            if !member {
                repair_one(state, &session, &channel).await;
            }
        }
    }
}

async fn repair_one(state: &Arc<DaemonState>, session: &Session, channel: &str) {
    let _lane = state.standing_sync.lock().await;
    if !admission_is_current(
        state,
        &session.pubkey,
        channel,
        session.runtime_generation,
        session.lifecycle_epoch,
        false,
    ) {
        tracing::debug!(
            pubkey = %session.pubkey,
            %channel,
            "running-standing repair was cancelled because its route is no longer current"
        );
        return;
    }
    let relay_parent = state.with_store(|store| store.channel_parent(channel).ok().flatten());
    let parent = crate::fabric::nip29::readiness::effective_parent_hint(
        relay_parent,
        (!session.readiness_parent.is_empty()).then_some(session.readiness_parent.as_str()),
        channel,
    );
    let confirmed = matches!(
        tokio::time::timeout(
            Duration::from_secs(15),
            state.snapshot().provider.ensure_channel_ready(
                crate::fabric::nip29::readiness::ChannelCtx {
                    channel,
                    expect_member: &session.pubkey,
                    parent_hint: parent.as_deref(),
                    name: None,
                },
            ),
        )
        .await,
        Ok(gate) if gate.is_ready()
    );
    if !confirmed {
        tracing::warn!(pubkey = %session.pubkey, %channel, "running session standing remains retryable");
        return;
    }
    match commit_confirmed_admission(
        state,
        &session.pubkey,
        channel,
        session.runtime_generation,
        session.lifecycle_epoch,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(pubkey = %session.pubkey, %channel, "running-standing repair became stale")
        }
        Err(error) => {
            tracing::error!(pubkey = %session.pubkey, %channel, %error, "running-standing repair persistence failed")
        }
    }
}
