use super::engine_lifecycle::pid_alive;
use super::*;
use std::collections::BTreeSet;

fn joined_channels_for_session(
    state: &Arc<DaemonState>,
    session_id: &str,
) -> Vec<(String, String)> {
    state.with_store(|s| {
        let Some(rec) = s.get_session(session_id).ok().flatten() else {
            return Vec::new();
        };
        let mut channels: BTreeSet<String> = s
            .list_session_joined_channels(&rec.session_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(channel, _)| channel)
            .collect();
        if !rec.channel_h.is_empty() {
            channels.insert(rec.channel_h.clone());
        }
        channels
            .into_iter()
            .map(|channel| (channel, rec.agent_pubkey.clone()))
            .collect()
    })
}

pub(in crate::daemon::server) fn remove_session_memberships(
    state: &Arc<DaemonState>,
    session_id: &str,
    reason: &'static str,
) {
    let removals = joined_channels_for_session(state, session_id);
    if removals.is_empty() {
        return;
    }
    state.with_store(|s| {
        for (channel, pubkey) in &removals {
            if let Err(e) = s.remove_channel_member(channel, pubkey) {
                tracing::error!(
                    session = %session_id,
                    channel,
                    pubkey = %pubkey_short(pubkey),
                    error = %e,
                    "membership cleanup: failed to remove local cache row"
                );
            }
        }
    });
    for (channel, pubkey) in removals {
        let provider = state.provider.clone();
        tokio::spawn(async move {
            tracing::info!(
                channel = %channel,
                pubkey = %pubkey_short(&pubkey),
                reason,
                "removing locally managed offline agent from channel"
            );
            provider.nip29_remove_member(&channel, &pubkey).await;
        });
    }
}

pub(in crate::daemon::server) fn cleanup_dead_local_sessions(state: &Arc<DaemonState>) {
    let now = now_secs();
    let stale_before = now.saturating_sub(3600);
    let stale: Vec<String> = state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|rec| {
                rec.child_pid.is_some_and(|pid| !pid_alive(pid))
                    || (!rec.working && rec.last_seen > 0 && rec.last_seen < stale_before)
            })
            .map(|rec| rec.session_id)
            .collect()
    });
    for session_id in stale {
        remove_session_memberships(state, &session_id, "startup-stale-pid");
        state.release_session_signer(&session_id);
        state.with_store(|s| {
            if let Err(e) = s.mark_dead(&session_id) {
                tracing::error!(session = %session_id, error = %e, "stale cleanup: failed to mark session dead");
            }
            if let Err(e) = s.mark_identity_dead_for_session(&session_id) {
                tracing::error!(session = %session_id, error = %e, "stale cleanup: failed to mark identity dead");
            }
        });
    }
}
