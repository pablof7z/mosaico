use super::super::super::*;

pub(in crate::daemon::server::demux) fn dispatch_all(
    state: &Arc<DaemonState>,
    event_id: &str,
    chat: &crate::domain::ChatMessage,
    hosted: &[String],
) -> bool {
    let mut dispatched = false;
    for mentioned_pk in chat
        .mentioned_pubkeys
        .iter()
        .filter(|pubkey| hosted.contains(pubkey))
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
    let body = chat.body.clone();
    let requester_pubkey = chat.from.pubkey.clone();
    tracing::info!(
        mentioned_pk = %crate::util::pubkey_short(&mentioned_pubkey),
        channel = %channel,
        "dispatching offline-agent-mention handler"
    );
    tokio::spawn(async move {
        super::handle(
            &st,
            &mentioned_pubkey,
            &channel,
            &body,
            Some(&requester_pubkey),
        )
        .await;
        complete(&st, &event_id, &mentioned_pubkey);
    });
}

fn complete(state: &Arc<DaemonState>, event_id: &str, mentioned_pubkey: &str) {
    if let Err(e) =
        state.with_store(|s| s.complete_offline_mention(event_id, mentioned_pubkey, now_secs()))
    {
        tracing::error!(
            event_id,
            mentioned_pk = %crate::util::pubkey_short(mentioned_pubkey),
            error = %e,
            "offline mention completion mark failed"
        );
    }
}
