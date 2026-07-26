use crate::state::Store;

pub(in crate::daemon::server::demux) fn has_alive_session_for(
    store: &Store,
    mentioned_pk: &str,
) -> bool {
    store
        .get_session(mentioned_pk)
        .ok()
        .flatten()
        .is_some_and(|session| session.is_running())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    #[test]
    fn alive_gate_is_runtime_ownership_not_channel_membership() {
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

        assert!(has_alive_session_for(&store, "durable-pk"));
        store
            .revoke_route_and_mark_absent("durable-pk", "channel-a", 2)
            .unwrap();
        assert!(
            has_alive_session_for(&store, "durable-pk"),
            "a direct mention can ring an owned live runtime after every explicit leave"
        );
    }
}
