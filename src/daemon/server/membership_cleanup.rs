use super::engine_lifecycle::pid_alive;
use super::*;
use std::collections::BTreeSet;

/// A locally-managed session that has neither a live child process nor a
/// heartbeat within this many seconds is treated as crashed/abandoned: its
/// channel memberships are torn down and the row is marked dead. 10 minutes —
/// long enough to ride out a transient stall, short enough that a crashed
/// session's public handle stops occupying a channel roster promptly.
pub(in crate::daemon::server) const STALE_MEMBERSHIP_SECS: u64 = 600;

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
    for (channel, pubkey) in removals {
        let state = state.clone();
        tokio::spawn(async move {
            tracing::info!(
                channel = %channel,
                pubkey = %pubkey_short(&pubkey),
                reason,
                "removing locally managed offline agent from channel"
            );
            let removed = state
                .provider
                .remove_member_confirmed(&channel, &pubkey)
                .await;
            if !removed.is_confirmed() {
                tracing::warn!(
                    channel = %channel,
                    pubkey = %pubkey_short(&pubkey),
                    reason,
                    outcome = ?removed,
                    "membership cleanup: relay removal was not confirmed; local membership row retained"
                );
            }
        });
    }
}

pub(in crate::daemon::server) fn cleanup_dead_local_sessions(state: &Arc<DaemonState>) {
    let now = now_secs();
    let stale_before = now.saturating_sub(STALE_MEMBERSHIP_SECS);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    fn register(state: &Arc<DaemonState>, external_id: &str, slug: &str, now: u64) -> String {
        state.with_store(|s| {
            s.register_session(&RegisterSession {
                harness: "claude".into(),
                external_id_kind: "harness_session".into(),
                external_id: external_id.into(),
                agent_pubkey: format!("pk-{external_id}"),
                agent_slug: slug.into(),
                channel_h: String::new(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now,
            })
            .expect("register test session")
        })
    }

    fn alive_ids(state: &Arc<DaemonState>) -> Vec<String> {
        state
            .with_store(|s| s.list_alive_sessions().unwrap_or_default())
            .into_iter()
            .map(|r| r.session_id)
            .collect()
    }

    /// A crashed session — no child pid, no heartbeat for longer than the TTL —
    /// is pruned once the periodic sweep runs, while a session seen inside the
    /// window survives.
    #[tokio::test]
    async fn crashed_session_is_pruned_after_ttl() {
        let state = DaemonState::new_for_test().await;
        let now = now_secs();

        let stale = register(&state, "stale", "reviewer", now);
        state.with_store(|s| {
            s.touch_session(&stale, now - (STALE_MEMBERSHIP_SECS + 60))
                .unwrap()
        });
        let fresh = register(&state, "fresh", "planner", now);

        cleanup_dead_local_sessions(&state);

        let alive = alive_ids(&state);
        assert!(
            !alive.contains(&stale),
            "a session silent for longer than the {STALE_MEMBERSHIP_SECS}s TTL must be pruned"
        );
        assert!(
            alive.contains(&fresh),
            "a session seen just now must be retained"
        );
    }

    /// A session whose last heartbeat is still inside the TTL window is not
    /// pruned — the sweep must not reap merely-idle sessions.
    #[tokio::test]
    async fn session_within_ttl_is_retained() {
        let state = DaemonState::new_for_test().await;
        let now = now_secs();

        let recent = register(&state, "recent", "reviewer", now);
        state.with_store(|s| {
            s.touch_session(&recent, now - (STALE_MEMBERSHIP_SECS - 60))
                .unwrap()
        });

        cleanup_dead_local_sessions(&state);

        assert!(alive_ids(&state).contains(&recent));
    }
}
