use super::super::*;

/// A durable (`perSessionKey: false`) agent has exactly one fixed-pubkey
/// identity, so a live session for it already IS "the agent" — there is no
/// second process to spawn. If `slug` is configured that way and its session
/// is currently running, return it so the caller can admit it into a channel
/// instead of attempting a conflicting fresh launch.
pub(in crate::daemon::server) fn running_durable_session(
    state: &Arc<DaemonState>,
    slug: &str,
) -> Option<crate::state::Session> {
    let home = crate::config::mosaico_home();
    let pubkey = crate::identity::keystore_entries(&home)
        .into_iter()
        .find(|entry| entry.slug == slug && !entry.per_session_key)
        .and_then(|entry| entry.pubkey)?;
    state
        .with_store(|store| store.get_session(&pubkey))
        .ok()
        .flatten()
        .filter(|rec| rec.is_running())
}

/// Passively admit an already-running session into `channel_h`: confirm
/// membership and reconcile subscriptions, without touching the process.
pub(in crate::daemon::server) async fn admit_running_agent(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    channel_h: &str,
) -> Result<()> {
    super::ensure_joinable(state, rec, channel_h).await?;
    sync_subscriptions(state).await
}

/// [`admit_running_agent`] for the orchestration path, which reports success
/// as a bool and logs rather than propagating the error.
pub(in crate::daemon::server) async fn admit_for_orchestration(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    channel_h: &str,
) -> bool {
    match admit_running_agent(state, rec, channel_h).await {
        Ok(()) => {
            tracing::info!(
                pubkey = %rec.pubkey,
                slug = %rec.agent_slug,
                child = %channel_h,
                "orchestration: durable agent already running elsewhere; admitted into channel"
            );
            true
        }
        Err(error) => {
            tracing::error!(
                pubkey = %rec.pubkey,
                slug = %rec.agent_slug,
                child = %channel_h,
                error = %format!("{error:#}"),
                "orchestration durable-agent admission failed"
            );
            false
        }
    }
}
