use super::resolution::work_root_for;
use super::*;

/// Add one pubkey as a channel member without disturbing existing rows. Reads the
/// current member set, appends, and re-materializes via `replace_channel_members`
/// (which preserves admins and won't demote an existing admin).
pub(in crate::daemon::server) fn add_channel_member(
    state: &Arc<DaemonState>,
    channel: &str,
    pubkey: &str,
) {
    state.with_store(|s| {
        let mut members: Vec<String> = s
            .list_channel_members(channel)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.role == "member")
            .map(|m| m.pubkey)
            .collect();
        if !members.iter().any(|p| p == pubkey) {
            members.push(pubkey.to_string());
        }
        s.replace_channel_members(channel, &members, now_secs())
            .ok();
    });
}

/// React to a subgroup add-agents orchestration event: authorize the signer,
/// provision the agents addressed to THIS backend, and either spawn fresh sessions
/// or resume exact prior sessions into the target channel.
pub(super) async fn handle_orchestration(
    state: &Arc<DaemonState>,
    event: &Event,
    op: crate::fabric::nip29::orchestration::AddAgentsOp,
) {
    use crate::fabric::nip29::orchestration::{adds_for_backend, is_authorized};

    let event_id = event.id.to_hex();
    let Some(backend_pk) = state.backend_pubkey().map(|s| s.to_string()) else {
        return;
    };
    let mine: Vec<_> = adds_for_backend(&op.adds, &backend_pk)
        .into_iter()
        .cloned()
        .collect();
    if mine.is_empty() {
        return;
    }

    let signer = event.pubkey.to_hex();
    let parent_roles = state.provider.fetch_group_roles(&op.parent).await;
    let authorized = is_authorized(&parent_roles, &signer) || {
        let child_roles = state.provider.fetch_group_roles(&op.child_h).await;
        is_authorized(&child_roles, &signer)
    };
    if !authorized {
        tracing::warn!(
            event_id = %&event_id[..event_id.len().min(8)],
            signer = %crate::util::pubkey_short(&signer),
            parent = %op.parent,
            child = %op.child_h,
            "orchestration rejected: signer is not an admin"
        );
        return;
    }

    if let Some(declared) = state.provider.fetch_group_parent(&op.child_h).await {
        if declared != op.parent && op.parent != op.child_h {
            tracing::warn!(
                event_id = %&event_id[..event_id.len().min(8)],
                child = %op.child_h,
                declared_parent = %declared,
                expected_parent = %op.parent,
                "orchestration refused: child declares a different parent"
            );
            return;
        }
    }

    let claimed = state.with_store(|s| {
        s.enqueue_inbox(&event_id, &backend_pk, &signer, &op.child_h, "", now_secs())
            .unwrap_or(false)
    });
    if !claimed {
        tracing::debug!(event_id = %&event_id[..event_id.len().min(8)], "orchestration already claimed by this backend — skipping");
        return;
    }

    let _ = ensure_subscription(state, &op.child_h).await;
    for target in &mine {
        if target.session_id.as_deref().is_some_and(|s| !s.is_empty()) {
            resume_target(state, &op, target).await;
        } else {
            spawn_target(state, &op, target).await;
        }
    }
}

async fn resume_target(
    state: &Arc<DaemonState>,
    op: &crate::fabric::nip29::orchestration::AddAgentsOp,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) {
    let session_id = target.session_id.as_deref().unwrap_or_default();
    let rec = match state
        .with_store(|s| s.get_session(session_id))
        .ok()
        .flatten()
        .or_else(|| {
            state
                .with_store(|s| s.find_session_by_prefix(session_id))
                .ok()
                .flatten()
        }) {
        Some(rec) => rec,
        None => {
            tracing::warn!(
                session_id,
                child = %op.child_h,
                "orchestration resume target is not known on this backend"
            );
            return;
        }
    };
    let Some(resume_id) = super::tmux_rpc::resume_token_for(&rec) else {
        tracing::warn!(
            session_id = %rec.session_id,
            child = %op.child_h,
            "orchestration resume target has no harness resume token"
        );
        return;
    };
    let work_root = state.with_store(|s| work_root_for(s, &op.child_h));
    match crate::tmux::resume_agent_in_channel(
        state,
        &rec.agent_slug,
        &work_root,
        &op.child_h,
        &resume_id,
    )
    .await
    {
        Ok(pane) => {
            tracing::info!(
                session_id = %rec.session_id,
                slug = %rec.agent_slug,
                child = %op.child_h,
                pane = %pane,
                "orchestration: session resumed"
            );
        }
        Err(e) => {
            tracing::error!(
                session_id = %rec.session_id,
                slug = %rec.agent_slug,
                child = %op.child_h,
                error = %e,
                "orchestration: session resume failed"
            );
        }
    }
}

async fn spawn_target(
    state: &Arc<DaemonState>,
    op: &crate::fabric::nip29::orchestration::AddAgentsOp,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) {
    let slug = &target.slug;
    let edge = config::edge_home();
    let id = match crate::identity::load_or_create(&edge, slug, now_secs()) {
        Ok(id) => {
            tracing::info!(slug = %slug, child = %op.child_h, "minting/loading agent identity for orchestration target");
            id
        }
        Err(e) => {
            tracing::error!(slug = %slug, error = %e, "failed to mint agent identity");
            return;
        }
    };
    let agent_pk = id.pubkey_hex();
    log_nip29_role_decision(
        &op.child_h,
        &agent_pk,
        "member",
        "handle_orchestration target agent durable pubkey",
    );

    let profile = DomainEvent::Profile(crate::domain::Profile {
        agent: crate::domain::AgentRef::new(agent_pk.clone(), slug.clone()),
        host: state.host.clone(),
        owners: state.owners.clone(),
        is_backend: false,
    });
    let _ = state.provider.publish(&profile, &id.keys).await;

    let mut confirmed = false;
    for attempt in 0..12u32 {
        let outcome = state
            .provider
            .nip29_add_member_outcome(&op.child_h, &agent_pk)
            .await;
        let (_, _, members) = state.provider.fetch_group_state(&op.child_h).await;
        if members.contains(&agent_pk) || (attempt > 0 && outcome.is_applied()) {
            confirmed = true;
            break;
        }
        if outcome.is_rejected() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    }
    if !confirmed {
        tracing::warn!(
            slug = %slug,
            child = %op.child_h,
            "member-add not confirmed after all retries — skipping spawn"
        );
        return;
    }
    add_channel_member(state, &op.child_h, &agent_pk);

    let work_root = state.with_store(|s| work_root_for(s, &op.child_h));
    match crate::tmux::spawn_agent(
        state,
        slug,
        &work_root,
        Vec::new(),
        None,
        Some(&op.child_h),
        None,
        None,
    )
    .await
    {
        Ok(pane) => {
            tracing::info!(slug = %slug, child = %op.child_h, pane = %pane, "orchestration: agent spawned");
        }
        Err(e) => {
            tracing::error!(slug = %slug, child = %op.child_h, error = %e, "orchestration: agent spawn failed");
        }
    }
}
