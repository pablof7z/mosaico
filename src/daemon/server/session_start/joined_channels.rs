use super::*;

pub(super) fn record(
    state: &Arc<DaemonState>,
    session_id: &str,
    primary: String,
    mut requested: Vec<String>,
    _now: u64,
) -> Vec<String> {
    requested.extend(
        state
            .with_store(|store| store.list_session_routes(session_id))
            .unwrap_or_default()
            .into_iter()
            .map(|(channel, _)| channel),
    );
    requested.retain(|channel| !channel.is_empty());
    if !primary.is_empty() {
        requested.push(primary);
    }
    requested.sort();
    requested.dedup();
    requested
}

pub(super) fn schedule_admission(
    state: Arc<DaemonState>,
    pubkey: String,
    runtime_generation: u64,
    lifecycle_epoch: u64,
    joined_channels: &[String],
    primary_channel: &str,
) {
    let passive = joined_channels
        .iter()
        .filter(|channel| channel.as_str() != primary_channel)
        .cloned()
        .collect::<Vec<_>>();
    if passive.is_empty() {
        return;
    }
    tokio::spawn(async move {
        for channel in passive {
            let _lane = state.standing_sync.lock().await;
            let outcome = state
                .provider
                .grant_member_confirmed(&channel, &pubkey)
                .await;
            if !outcome.is_confirmed() {
                tracing::warn!(pubkey, %channel, ?outcome, "session_start passive admission was not confirmed");
                continue;
            }
            match super::super::managed_lifecycle::commit_confirmed_admission(
                &state,
                &pubkey,
                &channel,
                runtime_generation,
                lifecycle_epoch,
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(pubkey, %channel, "confirmed passive admission became stale")
                }
                Err(error) => {
                    tracing::error!(pubkey, %channel, %error, "confirmed passive admission could not be persisted")
                }
            }
        }
        let _ = sync_subscriptions(&state).await;
    });
}

#[cfg(test)]
mod tests {
    use super::record;

    #[tokio::test]
    async fn unscoped_start_records_no_empty_channel_route() {
        let state = crate::daemon::server::DaemonState::new_for_test().await;
        assert!(record(&state, "pk", String::new(), vec![String::new()], 1).is_empty());
    }
}
