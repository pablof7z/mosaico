use super::super::resolution::work_root_for;
use super::super::*;
use std::sync::Arc;

mod claim;
pub(super) mod liveness;
mod target;
mod notice;

pub(super) use claim::dispatch_all;
use liveness::has_alive_session_for;

/// Spawn a local agent that was p-tagged in a kind:9 message but had no alive
/// session. The caller durably claims `(event_id, mentioned_pubkey)` before
/// entering this handler, so relay replay cannot repeat the side effect after a
/// daemon restart. `has_alive` still avoids unnecessary work on the first sight.
/// Delivery: session start schedules subscription/replay work in the daemon;
/// recent kind:9 events are re-materialized against the now-alive session and
/// delivered via `ring_doorbells`.
pub(super) async fn handle(
    state: &Arc<DaemonState>,
    event_id: &str,
    mentioned_pk: &str,
    channel: &str,
    body: &str,
    requester_pubkey: Option<&str>,
) {
    let has_alive = state.with_store(|s| has_alive_session_for(s, mentioned_pk, channel));
    if has_alive {
        tracing::debug!(
            mentioned_pk = %crate::util::pubkey_short(mentioned_pk),
            channel,
            "agent already has alive session - skipping spawn"
        );
        return;
    }

    let Some(target) = target::resolve_and_persist(
        state,
        event_id,
        mentioned_pk,
        channel,
        body,
        requester_pubkey,
    ) else {
        return;
    };
    let agent_slug = target.agent_slug;
    let target_session = target.session;

    let work_root = state.with_store(|s| work_root_for(s, channel));
    let has_path = state.with_store(|s| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(s)
            .path_for_channel(&work_root)
            .ok()
            .flatten()
            .is_some()
    });
    if !has_path {
        tracing::warn!(agent = %agent_slug, work_root = %work_root, channel, "no local channel root found - cannot spawn");
        return;
    }

    if let Some(target) = target_session.as_ref() {
        let resume_locator = match state.with_store(|s| s.native_resume_locator(mentioned_pk)) {
            Ok(locator) => locator,
            Err(e) => {
                tracing::error!(pubkey = %mentioned_pk, channel, error = %e, "exact mention resume lookup failed");
                return;
            }
        };
        if let Some(locator) = resume_locator {
            tracing::info!(
                agent = %agent_slug,
                pubkey = %mentioned_pk,
                channel,
                work_root = %work_root,
                "resuming exact session on mention"
            );
            match crate::session_host::resume_agent_in_channel(
                state,
                &agent_slug,
                &work_root,
                channel,
                &locator.locator_value,
                crate::session_host::LaunchIntent::Managed,
            )
            .await
            {
                Ok(endpoint_id) => tracing::info!(
                    agent = %agent_slug,
                    pubkey = %mentioned_pk,
                    endpoint = %endpoint_id,
                    channel,
                    "exact session resumed; pending inbox will ring its doorbell"
                ),
                Err(e) => {
                    tracing::warn!(
                        agent = %agent_slug,
                        pubkey = %mentioned_pk,
                        channel,
                        error = %e,
                        "exact session resume failed; mention remains pending"
                    );
                    state.emit_delivery_failure(
                        channel,
                        &agent_slug,
                        mentioned_pk,
                        format!("exact session resume failed; mention remains pending: {e:#}"),
                    );
                }
            }
            return;
        }

        let derived = match state.with_store(|s| s.is_derived_session_pubkey(mentioned_pk)) {
            Ok(derived) => derived,
            Err(e) => {
                tracing::error!(pubkey = %mentioned_pk, channel, error = %e, "exact mention signer lookup failed");
                return;
            }
        };
        if derived {
            tracing::warn!(
                agent = %agent_slug,
                pubkey = %mentioned_pk,
                channel,
                "per-session mention target has no native resume locator; mention remains pending"
            );
            state.emit_delivery_failure(
                channel,
                &agent_slug,
                mentioned_pk,
                "per-session target is not resumable; mention remains pending for the exact pubkey"
                    .to_string(),
            );
            return;
        }

        if target.agent_slug != agent_slug {
            tracing::error!(
                pubkey = %mentioned_pk,
                session_agent = %target.agent_slug,
                resolved_agent = %agent_slug,
                channel,
                "exact mention target agent projection disagrees; refusing launch"
            );
            return;
        }
    }

    let configured = match crate::identity::load(&crate::config::mosaico_home(), &agent_slug) {
        Ok(agent) => agent,
        Err(e) => {
            tracing::warn!(agent = %agent_slug, pubkey = %mentioned_pk, channel, error = %e, "cannot validate stable mention target");
            return;
        }
    };
    if configured.per_session_key || configured.pubkey_hex().as_deref() != Some(mentioned_pk) {
        tracing::warn!(
            agent = %agent_slug,
            pubkey = %mentioned_pk,
            channel,
            "offline p-tag is not the configured stable agent pubkey; refusing slug substitution"
        );
        return;
    }

    tracing::info!(agent = %agent_slug, pubkey = %mentioned_pk, channel, work_root = %work_root, "starting stable agent on exact mention");
    match crate::session_host::spawn_ephemeral_agent_for_pubkey(
        state,
        &agent_slug,
        &work_root,
        Some(channel),
        None,
        mentioned_pk,
    )
    .await
    {
        Ok(endpoint) => {
            tracing::info!(agent = %agent_slug, endpoint = %endpoint.endpoint_id, channel, "agent spawned successfully");
        }
        Err(e) => {
            tracing::warn!(agent = %agent_slug, channel, error = %e, "agent spawn failed");
            notice::publish_start_failure_notice(
                state,
                &agent_slug,
                &target_label(state, mentioned_pk, &agent_slug),
                channel,
                requester_pubkey,
                &e.to_string(),
            )
            .await;
        }
    }
}

fn target_label(state: &Arc<DaemonState>, pubkey: &str, fallback: &str) -> String {
    state
        .with_store(|s| {
            s.get_profile(pubkey)
                .ok()
                .flatten()
                .and_then(|p| (!p.name.is_empty()).then_some(p.name))
        })
        .unwrap_or_else(|| fallback.to_string())
}
