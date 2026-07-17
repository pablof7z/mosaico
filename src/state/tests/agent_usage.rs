use super::*;

fn session(pubkey: &str, agent_slug: &str, now: u64) -> RegisterSession {
    RegisterSession {
        pubkey: pubkey.into(),
        observed_harness: "codex".into(),
        agent_slug: agent_slug.into(),
        channel_h: "root".into(),
        child_pid: None,
        transcript_path: None,
        now,
    }
}

#[test]
fn aggregates_recent_sessions_by_canonical_agent_slug() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&session("old", "writer", 10))
        .unwrap();
    store
        .reserve_hook_session_for_test(&session("new-a", "writer", 80))
        .unwrap();
    store
        .reserve_hook_session_for_test(&session("new-b", "writer", 90))
        .unwrap();
    store
        .reserve_hook_session_for_test(&session("codex", "codex", 95))
        .unwrap();
    store
        .reserve_hook_session_for_test(&session("legacy", "legacy", 20))
        .unwrap();
    store.touch_session("new-a", 99).unwrap();

    let usage = store.agent_usage_since(50).unwrap();

    assert_eq!(
        usage,
        vec![
            AgentUsage {
                agent_slug: "codex".into(),
                recent_uses: 1,
                last_used: 95,
            },
            AgentUsage {
                agent_slug: "legacy".into(),
                recent_uses: 0,
                last_used: 20,
            },
            AgentUsage {
                agent_slug: "writer".into(),
                recent_uses: 2,
                last_used: 99,
            },
        ]
    );
}
