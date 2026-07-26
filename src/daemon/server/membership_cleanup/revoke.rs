use super::*;
use std::collections::BTreeSet;

pub(in crate::daemon::server) fn recorded_channels(
    state: &Arc<DaemonState>,
    pubkey: &str,
) -> Vec<String> {
    state.with_store(|store| {
        let Some(session) = store.get_session(pubkey).ok().flatten() else {
            return Vec::new();
        };
        let mut channels = store
            .list_session_routes(&session.pubkey)
            .unwrap_or_default()
            .into_iter()
            .map(|(channel, _)| channel)
            .collect::<BTreeSet<_>>();
        channels.extend(
            store
                .list_session_standing(&session.pubkey)
                .unwrap_or_default()
                .into_iter()
                .filter(|standing| standing.state != crate::state::StandingState::Absent)
                .map(|standing| standing.channel_h),
        );
        channels.into_iter().collect()
    })
}

/// Explicit operator destruction has no grace window. Attempt every recorded
/// channel even when the local membership mirror is stale, and await read-back.
pub(in crate::daemon::server) async fn remove_revoked_session_memberships(
    state: &Arc<DaemonState>,
    pubkey: &str,
    channels: Vec<String>,
) -> Vec<String> {
    let _lane = state.standing_sync.lock().await;
    let mut failures = Vec::new();
    for channel in channels {
        let public_channel = state.with_store(|store| public_channel_label(store, &channel));
        let standing = state
            .with_store(|store| store.get_session_standing(pubkey, &channel))
            .ok()
            .flatten();
        let outcome = state
            .provider
            .remove_member_confirmed(&channel, pubkey)
            .await;
        if !outcome.is_confirmed() {
            tracing::warn!(
                channel = %channel,
                ?outcome,
                "revoked-session membership removal was not confirmed"
            );
            failures.push(format!(
                "{public_channel}: membership removal was not confirmed"
            ));
        } else if let Some(standing) = standing {
            if let Err(error) = state.with_store(|store| {
                store.mark_member_standing_absent_if_epoch(
                    pubkey,
                    &channel,
                    standing.standing_epoch,
                    standing.session_lifecycle_epoch,
                    now_secs(),
                )
            }) {
                tracing::warn!(
                    channel = %channel,
                    error = %format!("{error:#}"),
                    "confirmed membership removal could not be persisted"
                );
                failures.push(format!(
                    "{public_channel}: confirmed membership removal could not be persisted"
                ));
            }
        }
    }
    failures
}

fn public_channel_label(store: &crate::state::Store, channel_h: &str) -> String {
    let path = crate::channel_ref::full_channel_ref(store, channel_h);
    if path.is_empty() {
        "channel with unavailable public path".to_string()
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    #[tokio::test]
    async fn targets_recorded_channels_when_membership_cache_is_empty() {
        let state = DaemonState::new_for_test().await;
        let session = "pk-operator-kill";
        state.with_store(|store| {
            store
                .reserve_hook_session_for_test(&RegisterSession {
                    pubkey: session.into(),
                    observed_harness: "claude-code".into(),
                    agent_slug: "reviewer".into(),
                    launch_channel_h: "active".into(),
                    work_root: "active".into(),
                    child_pid: None,
                    now: now_secs(),
                })
                .unwrap()
        });
        state
            .with_store(|store| store.grant_session_route(session, "joined", now_secs()))
            .unwrap();

        assert!(!state
            .with_store(|store| store.is_channel_member("active", "pk-operator-kill"))
            .unwrap());
        assert_eq!(
            recorded_channels(&state, session),
            vec![String::from("active"), String::from("joined")]
        );
    }

    #[test]
    fn cleanup_failure_labels_never_expose_internal_channel_ids() {
        let store = crate::state::Store::open_memory().unwrap();
        store.upsert_channel("root", "general", "", "", 1).unwrap();
        store
            .upsert_channel("opaque-child", "review", "", "root", 2)
            .unwrap();

        assert_eq!(public_channel_label(&store, "opaque-child"), "/root/review");
        assert_eq!(
            public_channel_label(&store, "unknown-internal-id"),
            "channel with unavailable public path"
        );
    }
}
