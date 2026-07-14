use super::engine_lifecycle::pid_alive;
use super::*;
use std::collections::BTreeSet;

mod revoke;
pub(in crate::daemon::server) use revoke::revoke_session_memberships;

/// Grace window after a locally managed session stops. During this window its
/// channel membership remains and its final idle status stays visible; after the
/// window, channel memberships are torn down.
pub(in crate::daemon::server) const STALE_MEMBERSHIP_SECS: u64 = 600;

fn joined_channels_for_session(state: &Arc<DaemonState>, pubkey: &str) -> Vec<(String, String)> {
    state.with_store(|s| {
        let Some(rec) = s.get_session(pubkey).ok().flatten() else {
            return Vec::new();
        };
        let mut channels: BTreeSet<String> = s
            .list_session_joined_channels(&rec.pubkey)
            .unwrap_or_default()
            .into_iter()
            .map(|(channel, _)| channel)
            .collect();
        if !rec.channel_h.is_empty() {
            channels.insert(rec.channel_h.clone());
        }
        channels
            .into_iter()
            .filter(|channel| s.is_channel_member(channel, &rec.pubkey).unwrap_or(false))
            .map(|channel| (channel, rec.pubkey.clone()))
            .collect()
    })
}

pub(in crate::daemon::server) fn remove_session_memberships(
    state: &Arc<DaemonState>,
    pubkey: &str,
    reason: &'static str,
) {
    let removals = joined_channels_for_session(state, pubkey);
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
    let candidates: Vec<(String, u64, bool, bool, bool)> = state.with_store(|s| {
        s.list_membership_cleanup_candidates(stale_before)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|rec| {
                let stale = rec.last_seen > 0 && rec.last_seen < stale_before;
                let process_dead = rec.alive && rec.child_pid.is_some_and(|pid| !pid_alive(pid));
                (stale || process_dead).then_some((
                    rec.pubkey,
                    rec.runtime_generation,
                    rec.alive,
                    stale,
                    process_dead,
                ))
            })
            .collect()
    });
    for (pubkey, runtime_generation, alive, stale, process_dead) in candidates {
        if stale {
            remove_session_memberships(state, &pubkey, "stale-membership");
            if let Err(e) = state
                .status
                .lock()
                .expect("status mutex poisoned")
                .forget_session(&pubkey)
            {
                tracing::error!(pubkey, error = %e, "stale cleanup: failed to forget status graph row");
            }
        }

        if alive && (stale || process_dead) {
            state.with_store(|s| {
                if let Err(e) = s.mark_dead_if_generation(&pubkey, runtime_generation) {
                    tracing::error!(pubkey, runtime_generation, error = %e, "stale cleanup: conditional teardown failed");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    fn register(state: &Arc<DaemonState>, pubkey: &str, slug: &str, now: u64) -> String {
        state.with_store(|s| {
            s.reserve_session(&RegisterSession {
                pubkey: pubkey.into(),
                harness: "claude".into(),
                agent_slug: slug.into(),
                channel_h: String::new(),
                child_pid: None,
                transcript_path: None,
                now,
            })
            .expect("register test session");
            pubkey.to_string()
        })
    }

    fn alive_ids(state: &Arc<DaemonState>) -> Vec<String> {
        state
            .with_store(|s| s.list_alive_sessions().unwrap_or_default())
            .into_iter()
            .map(|r| r.pubkey)
            .collect()
    }

    /// A crashed session — no child pid, no heartbeat for longer than the TTL —
    /// is pruned once the periodic sweep runs, while a session seen inside the
    /// window survives.
    #[tokio::test]
    async fn crashed_session_is_pruned_after_ttl() {
        let state = DaemonState::new_for_test().await;
        let now = now_secs();

        let stale = register(&state, "pk-stale", "reviewer", now);
        state.with_store(|s| {
            s.touch_session(&stale, now - (STALE_MEMBERSHIP_SECS + 60))
                .unwrap()
        });
        let fresh = register(&state, "pk-fresh", "planner", now);

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

        let recent = register(&state, "pk-recent", "reviewer", now);
        state.with_store(|s| {
            s.touch_session(&recent, now - (STALE_MEMBERSHIP_SECS - 60))
                .unwrap()
        });

        cleanup_dead_local_sessions(&state);

        assert!(alive_ids(&state).contains(&recent));
    }

    /// A cleanly ended session remains a channel member during the 10-minute grace
    /// window so its final idle status can stay visible.
    #[tokio::test]
    async fn dead_session_within_ttl_keeps_membership() {
        let state = DaemonState::new_for_test().await;
        let now = now_secs();
        let recent = register(&state, "pk-recent-dead", "reviewer", now);

        state
            .with_store(|s| {
                s.set_session_channel(&recent, "room")?;
                s.upsert_channel_member("room", "pk-recent-dead", "member", now)?;
                s.mark_dead(&recent)
            })
            .unwrap();

        cleanup_dead_local_sessions(&state);

        assert!(state
            .with_store(|s| s.is_channel_member("room", "pk-recent-dead"))
            .unwrap());
    }
}
