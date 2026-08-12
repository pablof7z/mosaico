use super::*;

pub(super) fn derive_and_emit_tail_events(state: &Arc<DaemonState>, event: &DomainEvent, now: u64) {
    match event {
        DomainEvent::Status(_) => {}
        DomainEvent::Profile(profile) => {
            let is_new = state
                .dedup
                .profiles
                .lock()
                .unwrap()
                .insert(profile.agent.pubkey.clone());
            if is_new {
                state.emit_tail(TailEvent::Profile {
                    ts: now,
                    agent: profile.agent.slug.clone(),
                    host: profile.host.clone(),
                    pubkey: profile.agent.pubkey.clone(),
                });
            }
        }
        DomainEvent::ChatMessage(chat) => {
            let from = if chat.from.slug.is_empty() {
                pubkey_short(&chat.from.pubkey)
            } else {
                chat.from.slug.clone()
            };
            let to = if chat.mentioned_pubkeys.is_empty() {
                "channel-chat".to_string()
            } else {
                chat.mentioned_pubkeys
                    .iter()
                    .map(|pubkey| pubkey_short(pubkey))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            state.emit_tail(TailEvent::Msg {
                ts: now,
                channel: chat.channel.clone(),
                from,
                to,
                body: chat.body.chars().take(200).collect(),
            });
        }
        DomainEvent::Reaction(_) => {}
    }
}

pub(super) fn derive_and_emit_status_tail_event(
    state: &Arc<DaemonState>,
    status: &crate::domain::Status,
    channel: &str,
    hosted: &[String],
    now: u64,
) {
    if hosted.contains(&status.agent.pubkey) {
        return;
    }
    let key = (status.agent.pubkey.clone(), channel.to_string());
    let is_new = {
        let mut sessions = state.dedup.peer_sessions.lock().unwrap();
        if sessions.contains_key(&key) {
            false
        } else {
            sessions.insert(
                key.clone(),
                PeerTracked {
                    first_seen: now,
                    channel: channel.to_string(),
                    slug: status.agent.slug.clone(),
                    host: status.host.clone(),
                },
            );
            true
        }
    };
    if is_new {
        state.emit_tail(TailEvent::Join {
            ts: now,
            channel: channel.to_string(),
            agent: status.agent.slug.clone(),
            host: status.host.clone(),
            session: status.agent.pubkey.clone(),
            rel_cwd: status.rel_cwd.clone(),
        });
    }

    let current = (status.title.clone(), status.state);
    let should_emit = {
        let mut statuses = state.dedup.last_status.lock().unwrap();
        if statuses.get(&key) == Some(&current) {
            false
        } else {
            statuses.insert(key, current);
            true
        }
    };
    if should_emit {
        state.emit_tail(TailEvent::Status {
            ts: now,
            channel: channel.to_string(),
            agent: status.agent.slug.clone(),
            text: status.title.clone(),
            state: status.state,
        });
    }
}
