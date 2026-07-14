use super::*;

fn recorded_channels(state: &Arc<DaemonState>, pubkey: &str) -> Vec<(String, String)> {
    state.with_store(|store| {
        let Some(session) = store.get_session(pubkey).ok().flatten() else {
            return Vec::new();
        };
        let mut channels = store
            .list_session_joined_channels(&session.pubkey)
            .unwrap_or_default()
            .into_iter()
            .map(|(channel, _)| channel)
            .collect::<BTreeSet<_>>();
        if !session.channel_h.is_empty() {
            channels.insert(session.channel_h.clone());
        }
        channels
            .into_iter()
            .map(|channel| (channel, session.pubkey.clone()))
            .collect()
    })
}

/// Explicit operator destruction has no grace window. Attempt every recorded
/// channel even when the local membership mirror is stale, and await read-back.
pub(in crate::daemon::server) async fn revoke_session_memberships(
    state: &Arc<DaemonState>,
    pubkey: &str,
) -> Vec<String> {
    let removals = recorded_channels(state, pubkey);
    let channels = removals
        .iter()
        .map(|(channel, _)| channel.clone())
        .collect::<Vec<_>>();
    let mut tasks = tokio::task::JoinSet::new();
    for (channel, pubkey) in removals {
        let state = state.clone();
        tasks.spawn(async move {
            let outcome = state
                .provider
                .remove_member_confirmed(&channel, &pubkey)
                .await;
            (channel, outcome)
        });
    }

    let mut failures = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((_channel, outcome)) if outcome.is_confirmed() => {}
            Ok((channel, outcome)) => failures.push(format!("{channel}: {outcome:?}")),
            Err(error) => failures.push(format!("cleanup task failed: {error}")),
        }
    }
    if failures.is_empty() {
        if let Err(error) = state.with_store(|store| -> Result<()> {
            store.set_session_channel(pubkey, "")?;
            for channel in channels {
                store.leave_session_channel(pubkey, &channel)?;
            }
            Ok(())
        }) {
            failures.push(format!("local channel cleanup: {error:#}"));
        }
    }
    failures
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
                .reserve_session(&RegisterSession {
                    pubkey: session.into(),
                    harness: "claude".into(),
                    agent_slug: "reviewer".into(),
                    channel_h: "active".into(),
                    child_pid: None,
                    transcript_path: None,
                    now: now_secs(),
                })
                .unwrap()
        });
        state
            .with_store(|store| store.join_session_channel(session, "joined", now_secs()))
            .unwrap();

        assert!(!state
            .with_store(|store| store.is_channel_member("active", "pk-operator-kill"))
            .unwrap());
        assert_eq!(
            recorded_channels(&state, session),
            vec![
                ("active".into(), "pk-operator-kill".into()),
                ("joined".into(), "pk-operator-kill".into()),
            ]
        );
    }
}
