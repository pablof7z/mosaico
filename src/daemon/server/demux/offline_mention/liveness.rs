use crate::state::Store;

pub(in crate::daemon::server::demux) fn has_alive_session_for(
    store: &Store,
    mentioned_pk: &str,
    channel: &str,
) -> bool {
    let Some(rec) = store.get_session(mentioned_pk).ok().flatten() else {
        return false;
    };
    if !rec.alive {
        return false;
    }
    if !store
        .is_derived_session_pubkey(mentioned_pk)
        .unwrap_or(true)
    {
        return true;
    }
    store
        .is_session_joined_channel(&rec.pubkey, channel)
        .unwrap_or(rec.channel_h == channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    #[test]
    fn durable_alive_gate_is_backend_global_across_channels() {
        let store = Store::open_memory().unwrap();
        store
            .reserve_session(&RegisterSession {
                pubkey: "durable-pk".into(),
                harness: "codex".into(),
                agent_slug: "chief".into(),
                channel_h: "channel-a".into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            })
            .unwrap();

        assert!(has_alive_session_for(&store, "durable-pk", "channel-b"));
    }
}
