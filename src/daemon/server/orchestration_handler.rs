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
    let parent_roles = match state.provider.fetch_group_roles(&op.parent).await {
        Ok(roles) => roles,
        Err(error) => {
            tracing::warn!(
                event_id = %&event_id[..event_id.len().min(8)],
                parent = %op.parent,
                error = %format!("{error:#}"),
                "orchestration rejected: parent admin state could not be verified"
            );
            return;
        }
    };
    let authorized = if is_authorized(&parent_roles, &signer) {
        true
    } else {
        let child_roles = match state.provider.fetch_group_roles(&op.child_h).await {
            Ok(roles) => roles,
            Err(error) => {
                tracing::warn!(
                    event_id = %&event_id[..event_id.len().min(8)],
                    child = %op.child_h,
                    error = %format!("{error:#}"),
                    "orchestration rejected: child admin state could not be verified"
                );
                return;
            }
        };
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

        let completed = if target
            .session_pubkey
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        {
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
    let session = target.session_pubkey.as_deref().unwrap_or("");
    format!(
        "orchestration:{backend_pk}:{target_index}:{}:{session}",
        target.slug
    )
}

fn orchestration_target_body(target: &crate::fabric::nip29::orchestration::AddTarget) -> String {
    match target.session_pubkey.as_deref().filter(|s| !s.is_empty()) {
        Some(session) => format!("resume {} {session}", target.slug),
        None => format!("spawn {}", target.slug),
    }
}

async fn resume_target(
    state: &Arc<DaemonState>,
    op: &crate::fabric::nip29::orchestration::AddAgentsOp,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) -> bool {
    let session_pubkey = target.session_pubkey.as_deref().unwrap_or_default();
    let rec = match state
        .with_store(|s| s.get_session(session_pubkey))
        .ok()
        .flatten()
    {
        Some(rec) => rec,
        None => {
            tracing::warn!(
                session_pubkey,
                child = %op.child_h,
                "orchestration resume target is not known on this backend"
            );
            return false;
        }
    };
    let Some(resume_id) = super::pty_rpc::resume_token_for(state, &rec) else {
        tracing::warn!(
            pubkey = %rec.pubkey,
            child = %op.child_h,
            "orchestration resume target has no harness resume token"
        );
        return false;
    };
    let work_root = match state.with_store(|store| work_root_for(store, &op.child_h)) {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(child = %op.child_h, %error, "orchestration resume workspace lookup failed");
            return false;
        }
    };
    match crate::session_host::resume_agent_in_channel(
        state,
        &rec,
        &work_root,
        &op.child_h,
        &resume_id,
        crate::session_host::LaunchIntent::Managed,
    )
    .await
    {
        Ok(pty_id) => {
            tracing::info!(
                pubkey = %rec.pubkey,
                slug = %rec.agent_slug,
                child = %op.child_h,
                pty_id = %pty_id,
                "orchestration: session resumed"
            );
            true
        }
        Err(e) => {
            tracing::error!(
                pubkey = %rec.pubkey,
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
    let work_root = match state.with_store(|s| work_root_for(s, &op.child_h)) {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(child = %op.child_h, %error, "orchestration spawn workspace lookup failed");
            return false;
        }
    };
    match crate::session_host::spawn_ephemeral_agent(
        state,
        slug,
        &work_root,
        Some(&op.child_h),
        None,
    )
    .await
    {
        Ok(endpoint) => {
            tracing::info!(slug = %slug, child = %op.child_h, endpoint = %endpoint.endpoint_id, "orchestration: agent spawned");
            true
        }
        Err(e) => {
            tracing::error!(slug = %slug, child = %op.child_h, error = %e, "orchestration: agent spawn failed");
            false
        }
    }
}
