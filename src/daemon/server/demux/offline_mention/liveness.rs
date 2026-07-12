use crate::state::Store;

pub(in crate::daemon::server::demux) fn has_alive_session_for(
    store: &Store,
    mentioned_pk: &str,
    channel: &str,
) -> bool {
    if store.is_durable_agent_pubkey(mentioned_pk).unwrap_or(false) {
        return store
            .live_durable_session_for_pubkey(mentioned_pk)
            .ok()
            .flatten()
            .is_some();
    }
    store
        .list_alive_sessions()
        .unwrap_or_default()
        .into_iter()
        .any(|rec| {
            rec.agent_pubkey == mentioned_pk
                && store
                    .is_session_joined_channel(&rec.session_id, channel)
                    .unwrap_or(rec.channel_h == channel)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    #[test]
    fn durable_alive_gate_is_backend_global_across_channels() {
        let store = Store::open_memory().unwrap();
        store
            .claim_durable_agent_session("durable-pk", "chief", "sid", 1)
            .unwrap();
        store
            .upsert_session_row(
                "sid",
                &RegisterSession {
                    harness: "codex".into(),
                    external_id_kind: "harness_session".into(),
                    external_id: "native".into(),
                    agent_pubkey: "durable-pk".into(),
                    agent_slug: "chief".into(),
                    channel_h: "channel-a".into(),
                    child_pid: None,
                    transcript_path: None,
                    resume_id: String::new(),
                    now: 1,
                },
            )
            .unwrap();

        assert!(has_alive_session_for(&store, "durable-pk", "channel-b"));
    }
}
