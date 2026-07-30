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
        if begin(state, &claim.event_id, &claim.mentioned_pubkey, &chat) {
            tracing::info!(
                event_id = %claim.event_id,
                mentioned_pk = %crate::util::pubkey_short(&claim.mentioned_pubkey),
                "retrying durable offline mention recovery"
            );
            dispatch(state, &claim.event_id, &chat, &claim.mentioned_pubkey);
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
        if begin(state, event_id, mentioned_pk, chat) {
            dispatch(state, event_id, chat, mentioned_pk);
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
) -> bool {
    match state.with_store(|s| {
        s.claim_offline_mention(
            event_id,
            mentioned_pubkey,
            &chat.from.pubkey,
            &chat.channel,
            &chat.body,
            now_secs(),
        )
    }) {
        Ok(claimed) => claimed,
        Err(e) => {
            tracing::error!(
                event_id,
                mentioned_pk = %crate::util::pubkey_short(mentioned_pubkey),
                error = %e,
                "offline mention claim failed; refusing duplicate-prone dispatch"
            );
            false
        }
    }
}

fn dispatch(
    state: &Arc<DaemonState>,
    event_id: &str,
    chat: &crate::domain::ChatMessage,
    mentioned_pubkey: &str,
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
        finish(&st, &event_id, &mentioned_pubkey, outcome);
    });
}

fn finish(
    state: &Arc<DaemonState>,
    event_id: &str,
    mentioned_pubkey: &str,
    outcome: RecoveryOutcome,
) {
    let result =
        match outcome {
            RecoveryOutcome::Complete => state
                .with_store(|s| s.complete_offline_mention(event_id, mentioned_pubkey, now_secs())),
            RecoveryOutcome::Retry => state
                .with_store(|s| s.retry_offline_mention(event_id, mentioned_pubkey, now_secs())),
        };
    if let Err(e) = result {
        tracing::error!(
            event_id,
            mentioned_pk = %crate::util::pubkey_short(mentioned_pubkey),
            outcome = ?outcome,
            error = %e,
            "offline mention claim state update failed"
        );
    }
}
