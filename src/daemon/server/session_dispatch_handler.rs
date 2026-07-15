use super::*;

/// React to a cross-backend session dispatch event targeted to this backend.
pub(super) async fn handle_session_dispatch(
    state: &Arc<DaemonState>,
    event: &Event,
    op: crate::fabric::nip29::session_dispatch::SessionDispatchOp,
) {
    let event_id = event.id.to_hex();
    let Some(backend_pk) = state.backend_pubkey() else {
        return;
    };
    if op.target.backend_pubkey != backend_pk {
        return;
    }

    let signer = event.pubkey.to_hex();
    let (_exists, roles, members) = match state.provider.fetch_group_state(&op.route_channel).await
    {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                event_id = %&event_id[..event_id.len().min(8)],
                channel = %op.route_channel,
                error = %format!("{error:#}"),
                "session dispatch rejected: route-channel membership could not be verified"
            );
            return;
        }
    };
    if !roles.contains_key(&signer) && !members.contains(&signer) {
        tracing::warn!(
            event_id = %&event_id[..event_id.len().min(8)],
            signer = %crate::util::pubkey_short(&signer),
            channel = %op.route_channel,
            "session dispatch rejected: signer is not a route-channel member"
        );
        return;
    }

    let target_key = format!("session-dispatch:{backend_pk}:{}", op.target.slug);
    let body = format!(
        "dispatch {} in {} on {}",
        op.target.slug,
        op.target.workspace,
        op.target.channels.join(", ")
    );
    let claimed = match state.with_store(|s| {
        s.claim_orchestration_target(
            &event_id,
            &target_key,
            &signer,
            &op.route_channel,
            &body,
            now_secs(),
        )
    }) {
        Ok(claimed) => claimed,
        Err(e) => {
            tracing::error!(event_id = %&event_id[..event_id.len().min(8)], error = %e, "session dispatch claim failed");
            return;
        }
    };
    if !claimed {
        tracing::debug!(event_id = %&event_id[..event_id.len().min(8)], target = %target_key, "session dispatch target already claimed");
        return;
    }

    let completed = spawn_dispatched(state, &event_id, &op).await;
    let finish = if completed {
        state.with_store(|s| s.complete_orchestration_target(&event_id, &target_key, now_secs()))
    } else {
        state.with_store(|s| s.retry_orchestration_target(&event_id, &target_key))
    };
    if let Err(e) = finish {
        tracing::error!(event_id = %&event_id[..event_id.len().min(8)], target = %target_key, error = %e, "session dispatch state update failed");
    }
}

async fn spawn_dispatched(
    state: &Arc<DaemonState>,
    event_id: &str,
    op: &crate::fabric::nip29::session_dispatch::SessionDispatchOp,
) -> bool {
    let slug = &op.target.slug;
    let mosaico_home = config::mosaico_home();
    if let Err(e) = crate::identity::load(&mosaico_home, slug) {
        tracing::error!(slug = %slug, error = %e, "session dispatch: failed to mint agent identity");
        return false;
    }

    match crate::session_host::spawn_dispatched_ephemeral_agent(
        state,
        slug,
        &op.target.workspace,
        &op.target.channels,
        event_id,
    )
    .await
    {
        Ok(spawned) => {
            grant_joined_channels(state, &spawned.pubkey, &op.target.channels).await;
            tracing::info!(
                slug = %slug,
                workspace = %op.target.workspace,
                pty_id = %spawned.pty_id,
                pubkey = %spawned.pubkey,
                "session dispatch: agent spawned"
            );
            true
        }
        Err(e) => {
            tracing::error!(slug = %slug, workspace = %op.target.workspace, error = %e, "session dispatch: spawn failed");
            false
        }
    }
}

async fn grant_joined_channels(state: &Arc<DaemonState>, pubkey: &str, channels: &[String]) {
    let Some(rec) = state.with_store(|s| s.get_session(pubkey).ok().flatten()) else {
        return;
    };
    for channel in channels {
        let _ = ensure_subscription(state, channel).await;
        let confirmed = state
            .provider
            .grant_member_confirmed(channel, &rec.pubkey)
            .await;
        if !confirmed.is_confirmed() {
            tracing::warn!(pubkey, channel = %channel, "session dispatch: membership was not confirmed");
        }
        state.with_store(|s| {
            let _ = s.join_session_channel(pubkey, channel, now_secs());
        });
    }
    state.outbox_notify.notify_waiters();
}
