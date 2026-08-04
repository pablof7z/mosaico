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
    // The relay-signed roster is read from the cache the ONE retained
    // group-records observation keeps current — no wire call on the dispatch
    // path. `is_channel_member` is true for either list, which is the same
    // admission this used to spell as "named by 39001 or by 39002".
    let member = match state.with_store(|s| s.is_channel_member(&op.route_channel, &signer)) {
        Ok(member) => member,
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
    if !member {
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
            tracing::info!(
                slug = %slug,
                workspace = %op.target.workspace,
                endpoint = %spawned.endpoint.endpoint_id,
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
