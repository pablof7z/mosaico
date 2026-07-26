use crate::state::Store;

pub(in crate::daemon::server::demux) fn has_alive_session_for(
    store: &Store,
    mentioned_pk: &str,
    channel: &str,
) -> bool {
    let Some(rec) = store.get_session(mentioned_pk).ok().flatten() else {
        return false;
    };
    if !rec.is_running() {
        return false;
    }
    store
        .has_session_route(&rec.pubkey, channel)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    #[test]
    fn alive_gate_requires_membership_in_the_delivery_channel() {
        let store = Store::open_memory().unwrap();
        store
            .reserve_hook_session_for_test(&RegisterSession {
                pubkey: "durable-pk".into(),
                observed_harness: "codex".into(),
                agent_slug: "chief".into(),
                launch_channel_h: "channel-a".into(),
                work_root: "channel-a".into(),
                child_pid: None,
                now: 1,
            })
            .unwrap();

        assert!(has_alive_session_for(&store, "durable-pk", "channel-a"));
        assert!(!has_alive_session_for(&store, "durable-pk", "channel-b"));
    }
}
