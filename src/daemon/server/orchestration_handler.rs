use super::resolution::work_root_for;
use super::*;

/// React to a subgroup add-agents orchestration event: authorize the signer,
/// provision the agents addressed to THIS backend, and either spawn fresh sessions
/// or resume exact prior sessions into the target channel.
pub(super) async fn handle_orchestration(
    state: &Arc<DaemonState>,
    event: &Event,
    op: crate::fabric::nip29::orchestration::AddAgentsOp,
) {
    use crate::fabric::nip29::orchestration::is_authorized;

    let event_id = event.id.to_hex();
    let Some(backend_pk) = state.backend_pubkey() else {
        return;
    };
    let mine: Vec<_> = op
        .adds
        .iter()
        .enumerate()
        .filter(|(_, add)| add.backend_pubkey == backend_pk)
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

    let _ = ensure_subscription(state, &op.child_h).await;
    for (target_index, target) in mine {
        let target_key = orchestration_target_key(&backend_pk, target_index, target);
        let body = orchestration_target_body(target);
        let claimed = match state.with_store(|s| {
            s.claim_orchestration_target(
                &event_id,
                &target_key,
                &signer,
                &op.child_h,
                &body,
                now_secs(),
            )
        }) {
            Ok(claimed) => claimed,
            Err(e) => {
                tracing::error!(
                    event_id = %&event_id[..event_id.len().min(8)],
                    target = %target_key,
                    error = %e,
                    "orchestration target claim failed"
                );
                continue;
            }
        };
        if !claimed {
            tracing::debug!(
                event_id = %&event_id[..event_id.len().min(8)],
                target = %target_key,
                "orchestration target already complete or in-flight — skipping"
            );
            continue;
        }

        let completed = if target.session_id.as_deref().is_some_and(|s| !s.is_empty()) {
            resume_target(state, &op, target).await
        } else {
            spawn_target(state, &op, target).await
        };
        let finish_result = if completed {
            state
                .with_store(|s| s.complete_orchestration_target(&event_id, &target_key, now_secs()))
        } else {
            state.with_store(|s| s.retry_orchestration_target(&event_id, &target_key))
        };
        if let Err(e) = finish_result {
            tracing::error!(
                event_id = %&event_id[..event_id.len().min(8)],
                target = %target_key,
                error = %e,
                "orchestration target state update failed"
            );
        }
    }
}

fn orchestration_target_key(
    backend_pk: &str,
    target_index: usize,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) -> String {
    let session = target.session_id.as_deref().unwrap_or("");
    format!(
        "orchestration:{backend_pk}:{target_index}:{}:{session}",
        target.slug
    )
}

fn orchestration_target_body(target: &crate::fabric::nip29::orchestration::AddTarget) -> String {
    match target.session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(session) => format!("resume {} {session}", target.slug),
        None => format!("spawn {}", target.slug),
    }
}

async fn resume_target(
    state: &Arc<DaemonState>,
    op: &crate::fabric::nip29::orchestration::AddAgentsOp,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) -> bool {
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
            return false;
        }
    };
    let Some(resume_id) = super::pty_rpc::resume_token_for(&rec) else {
        tracing::warn!(
            session_id = %rec.session_id,
            child = %op.child_h,
            "orchestration resume target has no harness resume token"
        );
        return false;
    };
    let work_root = state.with_store(|s| work_root_for(s, &op.child_h));
    match crate::session_host::resume_agent_in_channel(
        state,
        &rec.agent_slug,
        &work_root,
        &op.child_h,
        &resume_id,
    )
    .await
    {
        Ok(pty_id) => {
            tracing::info!(
                session_id = %rec.session_id,
                slug = %rec.agent_slug,
                child = %op.child_h,
                pty_id = %pty_id,
                "orchestration: session resumed"
            );
            true
        }
        Err(e) => {
            tracing::error!(
                session_id = %rec.session_id,
                slug = %rec.agent_slug,
                child = %op.child_h,
                error = %e,
                "orchestration: session resume failed"
            );
            false
        }
    }
}

async fn spawn_target(
    state: &Arc<DaemonState>,
    op: &crate::fabric::nip29::orchestration::AddAgentsOp,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) -> bool {
    let slug = &target.slug;
    let edge = config::edge_home();
    let id = match crate::identity::load_or_create(&edge, slug, now_secs()) {
        Ok(id) => {
            tracing::info!(slug = %slug, child = %op.child_h, "loading local derivation root for orchestration target");
            id
        }
        Err(e) => {
            tracing::error!(slug = %slug, error = %e, "failed to mint agent identity");
            return false;
        }
    };
    drop(id);

    let work_root = state.with_store(|s| work_root_for(s, &op.child_h));
    match crate::session_host::spawn_ephemeral_agent(
        state,
        slug,
        &work_root,
        Vec::new(),
        None,
        Some(&op.child_h),
        None,
    )
    .await
    {
        Ok(pty_id) => {
            tracing::info!(slug = %slug, child = %op.child_h, pty_id = %pty_id, "orchestration: agent spawned");
            true
        }
        Err(e) => {
            tracing::error!(slug = %slug, child = %op.child_h, error = %e, "orchestration: agent spawn failed");
            false
        }
    }
}
