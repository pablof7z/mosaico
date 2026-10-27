use super::super::super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::daemon::server::demux) enum RecoveryOutcome {
    Complete,
    Retry,
}

const RETRY_BATCH_LIMIT: u32 = 64;

pub(in crate::daemon::server::demux) fn drive_retries(state: &Arc<DaemonState>) {
    let claims = match state
        .with_store(|store| store.list_retryable_offline_mentions(now_secs(), RETRY_BATCH_LIMIT))
    {
        Ok(claims) => claims,
        Err(error) => {
            tracing::error!(%error, "offline mention retry scan failed");
            return;
        }
    };
    for claim in claims {
        let chat = crate::domain::ChatMessage {
            from: crate::domain::AgentRef::new(claim.from_pubkey, String::new()),
            channel: claim.channel_h,
            body: claim.body,
            mentioned_pubkeys: vec![claim.mentioned_pubkey.clone()],
            attachments: Vec::new(),
        };
        if let Some(generation) = begin(state, &claim.event_id, &claim.mentioned_pubkey, &chat) {
            tracing::info!(
                event_id = %claim.event_id,
                mentioned_pk = %crate::util::pubkey_short(&claim.mentioned_pubkey),
                "retrying durable offline mention recovery"
            );
            dispatch(
                state,
                &claim.event_id,
                &chat,
                &claim.mentioned_pubkey,
                generation,
            );
        }
    }
}

pub(in crate::daemon::server::demux) fn dispatch_all(
    state: &Arc<DaemonState>,
    event_id: &str,
    chat: &crate::domain::ChatMessage,
    owned_targets: &[String],
) -> bool {
    let mut dispatched = false;
    for mentioned_pk in chat
        .mentioned_pubkeys
        .iter()
        .filter(|pubkey| owned_targets.contains(pubkey))
    {
        if let Some(generation) = begin(state, event_id, mentioned_pk, chat) {
            dispatch(state, event_id, chat, mentioned_pk, generation);
            dispatched = true;
        }
    }
    dispatched
}

fn begin(
    state: &Arc<DaemonState>,
    event_id: &str,
    mentioned_pubkey: &str,
    chat: &crate::domain::ChatMessage,
) -> Option<u32> {
    match state.with_store(|s| {
        s.claim_offline_mention_run(
            event_id,
            mentioned_pubkey,
            &chat.from.pubkey,
            &chat.channel,
            &chat.body,
            now_secs(),
        )
    }) {
        Ok(generation) => generation,
        Err(e) => {
            tracing::error!(
                event_id,
                mentioned_pk = %crate::util::pubkey_short(mentioned_pubkey),
                error = %e,
                "offline mention claim failed; refusing duplicate-prone dispatch"
            );
            None
        }
    }
}

fn dispatch(
    state: &Arc<DaemonState>,
    event_id: &str,
    chat: &crate::domain::ChatMessage,
    mentioned_pubkey: &str,
    generation: u32,
) {
    let st = state.clone();
    let event_id = event_id.to_string();
    let mentioned_pubkey = mentioned_pubkey.to_string();
    let channel = chat.channel.clone();
    let requester_pubkey = chat.from.pubkey.clone();
    tracing::info!(
        mentioned_pk = %crate::util::pubkey_short(&mentioned_pubkey),
        channel = %channel,
        "dispatching offline-agent-mention handler"
    );
    tokio::spawn(async move {
        let outcome =
            super::handle(&st, &mentioned_pubkey, &channel, Some(&requester_pubkey)).await;
        finish(&st, &event_id, &mentioned_pubkey, generation, outcome);
    });
}

/// Record a recovery run's outcome. A run superseded by a lease takeover
/// changes nothing, except that its success still wins over a charge.
fn finish(
    state: &Arc<DaemonState>,
    event_id: &str,
    mentioned_pubkey: &str,
    generation: u32,
    outcome: RecoveryOutcome,
) {
    let now = now_secs();
    let result = state.with_store(|s| match outcome {
        RecoveryOutcome::Complete => s
            .complete_offline_mention_run(event_id, mentioned_pubkey, generation, now)
            .map(|completed| (completed, None)),
        RecoveryOutcome::Retry => {
            if !s.owns_offline_mention(event_id, mentioned_pubkey, generation)? {
                return Ok((false, None));
            }
            s.retry_offline_mention(event_id, mentioned_pubkey, now)
                .map(|retry| (true, retry))
        }
    });
    match result {
        Ok((false, _)) => tracing::warn!(
            event_id,
            mentioned_pk = %crate::util::pubkey_short(mentioned_pubkey),
            generation,
            ?outcome,
            "superseded offline mention recovery finished; outcome ignored"
        ),
        // Exhaustion is logged by the store.
        Ok((true, Some(crate::state::ClaimRetry::Scheduled { .. }))) => {
            super::super::retry_scheduled(state);
        }
        Ok(_) => {}
        Err(e) => tracing::error!(
            event_id,
            mentioned_pk = %crate::util::pubkey_short(mentioned_pubkey),
            outcome = ?outcome,
            error = %e,
            "offline mention claim state update failed"
        ),
    }
}

#[cfg(test)]
pub(in crate::daemon::server::demux) fn finish_for_test(
    state: &Arc<DaemonState>,
    event_id: &str,
    mentioned_pubkey: &str,
    generation: u32,
    completed: bool,
) {
    let outcome = if completed {
        RecoveryOutcome::Complete
    } else {
        RecoveryOutcome::Retry
    };
    finish(state, event_id, mentioned_pubkey, generation, outcome);
}
