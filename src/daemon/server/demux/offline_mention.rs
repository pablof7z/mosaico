use super::super::resolution::work_root_for;
use super::super::*;
use std::sync::Arc;

mod claim;
pub(super) mod liveness;
mod notice;
mod standing;
mod target;

use claim::RecoveryOutcome;
pub(super) use claim::{dispatch_all, drive_retries};
use liveness::has_alive_session_for;

/// Spawn a local agent that was p-tagged in a kind:9 message but had no running
/// session. The caller durably claims `(event id, mentioned_pubkey)` before
/// entering this handler, so relay replay cannot repeat the side effect after a
/// daemon restart. `has_alive` still avoids unnecessary work on the first sight.
/// The canonical direct router has already parked the inbox row; successful
/// recovery only supplies an executor, whose doorbell claims that existing row.
pub(super) async fn handle(
    state: &Arc<DaemonState>,
    mentioned_pk: &str,
    channel: &str,
    requester_pubkey: Option<&str>,
) -> RecoveryOutcome {
    let has_alive = state.with_store(|s| has_alive_session_for(s, mentioned_pk));
    if has_alive {
        return RecoveryOutcome::Complete;
    }

    let target = match target::resolve(state, mentioned_pk, channel) {
        target::Resolution::Ready(target) => target,
        target::Resolution::Retry => return RecoveryOutcome::Retry,
    };
    let agent_slug = target.agent_slug;
    let target_session = target.session;

    let (work_root, launch_channel, mention_route_active) = match target_session.as_ref() {
        Some(session) => {
            let routes = match state.with_store(|store| store.list_session_routes(mentioned_pk)) {
                Ok(routes) => routes
                    .into_iter()
                    .map(|(channel, _)| channel)
                    .collect::<Vec<_>>(),
                Err(error) => {
                    tracing::error!(pubkey = %mentioned_pk, channel, %error, "mention recovery route lookup failed");
                    return RecoveryOutcome::Retry;
                }
            };
            (
                session.work_root.clone(),
                routes.first().cloned(),
                routes.iter().any(|route| route == channel),
            )
        }
        None => {
            let root = match state.with_store(|store| work_root_for(store, channel)) {
                Ok(root) => root,
                Err(error) => {
                    tracing::error!(channel, %error, "mention spawn workspace ancestry lookup failed");
                    return RecoveryOutcome::Retry;
                }
            };
            (root, Some(channel.to_string()), true)
        }
    };
    let has_path = match state.with_store(|s| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(s).path_for_channel(&work_root)
    }) {
        Ok(path) => path.is_some(),
        Err(error) => {
            tracing::error!(channel, work_root, %error, "mention spawn workspace path lookup failed");
            return RecoveryOutcome::Retry;
        }
    };
    if !has_path {
        tracing::warn!(agent = %agent_slug, work_root = %work_root, channel, "no local channel root found - cannot spawn");
        return RecoveryOutcome::Retry;
    }

    if let Some(target) = target_session.as_ref() {
        let resume_locator = match state
            .with_store(|s| s.native_resume_locator(mentioned_pk, &target.observed_harness))
        {
            Ok(locator) => locator,
            Err(e) => {
                tracing::error!(pubkey = %mentioned_pk, channel, error = %e, "exact mention resume lookup failed");
                return RecoveryOutcome::Retry;
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
            return match crate::session_host::resume_agent(
                state,
                target,
                &locator.locator_value,
                crate::session_host::ResumeRequest::without_args(
                    crate::session_host::LaunchIntent::Managed,
                ),
            )
            .await
            {
                Ok(endpoint_id) => {
                    tracing::info!(
                        agent = %agent_slug,
                        pubkey = %mentioned_pk,
                        endpoint = %endpoint_id,
                        channel,
                        "exact session resumed; pending inbox will ring its doorbell"
                    );
                    finish_recovery(state, mentioned_pk, channel, mention_route_active).await
                }
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
                    RecoveryOutcome::Retry
                }
            };
        }

        let derived = match state.with_store(|s| s.is_derived_session_pubkey(mentioned_pk)) {
            Ok(derived) => derived,
            Err(e) => {
                tracing::error!(pubkey = %mentioned_pk, channel, error = %e, "exact mention signer lookup failed");
                return RecoveryOutcome::Retry;
            }
        };
        if derived {
            tracing::info!(
                agent = %agent_slug,
                pubkey = %mentioned_pk,
                channel,
                "per-session mention target has no native resume locator; attempting exact fresh relaunch"
            );
            return match crate::session_host::spawn_ephemeral_agent_for_pubkey(
                state,
                &agent_slug,
                &work_root,
                launch_channel.as_deref(),
                None,
                mentioned_pk,
            )
            .await
            {
                Ok(endpoint) => {
                    tracing::info!(
                        agent = %agent_slug,
                        pubkey = %mentioned_pk,
                        endpoint = %endpoint.endpoint_id,
                        channel,
                        "session relaunched with its exact pubkey"
                    );
                    finish_recovery(state, mentioned_pk, channel, mention_route_active).await
                }
                Err(e) => {
                    tracing::warn!(
                        agent = %agent_slug,
                        pubkey = %mentioned_pk,
                        channel,
                        error = %e,
                        "exact fresh relaunch refused or failed; mention remains pending"
                    );
                    state.emit_delivery_failure(
                        channel,
                        &agent_slug,
                        mentioned_pk,
                        format!(
                            "exact fresh relaunch refused or failed; mention remains pending: {e:#}"
                        ),
                    );
                    RecoveryOutcome::Retry
                }
            };
        }

        if target.agent_slug != agent_slug {
            tracing::error!(
                pubkey = %mentioned_pk,
                session_agent = %target.agent_slug,
                resolved_agent = %agent_slug,
                channel,
                "exact mention target agent projection disagrees; refusing launch"
            );
            return RecoveryOutcome::Retry;
        }
    }

    let configured = match crate::identity::load(&crate::config::mosaico_home(), &agent_slug) {
        Ok(agent) => agent,
        Err(e) => {
            tracing::warn!(agent = %agent_slug, pubkey = %mentioned_pk, channel, error = %e, "cannot validate stable mention target");
            return RecoveryOutcome::Retry;
        }
    };
    if configured.per_session_key || configured.pubkey_hex().as_deref() != Some(mentioned_pk) {
        tracing::warn!(
            agent = %agent_slug,
            pubkey = %mentioned_pk,
            channel,
            "offline p-tag is not the configured stable agent pubkey; refusing slug substitution"
        );
        return RecoveryOutcome::Retry;
    }

    tracing::info!(agent = %agent_slug, pubkey = %mentioned_pk, channel, work_root = %work_root, "starting stable agent on exact mention");
    match crate::session_host::spawn_ephemeral_agent_for_pubkey(
        state,
        &agent_slug,
        &work_root,
        launch_channel.as_deref(),
        None,
        mentioned_pk,
    )
    .await
    {
        Ok(endpoint) => {
            tracing::info!(agent = %agent_slug, endpoint = %endpoint.endpoint_id, channel, "agent spawned successfully");
            finish_recovery(state, mentioned_pk, channel, mention_route_active).await
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
            RecoveryOutcome::Retry
        }
    }
}

async fn finish_recovery(
    state: &Arc<DaemonState>,
    pubkey: &str,
    channel: &str,
    mention_route_active: bool,
) -> RecoveryOutcome {
    if !mention_route_active {
        return RecoveryOutcome::Complete;
    }
    match standing::confirm(state, pubkey, channel).await {
        Ok(()) => RecoveryOutcome::Complete,
        Err(error) => {
            tracing::warn!(%pubkey, channel, %error, "recovered exact target is not yet relay-admitted");
            RecoveryOutcome::Retry
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
