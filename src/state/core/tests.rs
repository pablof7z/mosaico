use super::*;

fn reg(pubkey: &str, channel: &str, now: u64) -> RegisterSession {
    RegisterSession {
        pubkey: pubkey.into(),
        observed_harness: "codex".into(),
        agent_slug: "agent".into(),
        channel_h: channel.into(),
        child_pid: None,
        transcript_path: None,
        now,
    }
}

#[test]
fn table_samples_prefer_alive_sessions_and_locators() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&reg("alive", "room", 100))
        .unwrap();
    store
        .reserve_hook_session_for_test(&reg("dead", "room", 200))
        .unwrap();
    store
        .mark_runtime_stopped("dead", StopReason::Unknown, 201)
        .unwrap();
    store
        .put_session_locator("codex", LOCATOR_PTY, "alive-endpoint", "alive", 100)
        .unwrap();
    store
        .put_session_locator("codex", LOCATOR_PTY, "dead-endpoint", "dead", 200)
        .unwrap();

    let sessions = store
        .application_table_sample_rows("sessions", &["pubkey"], 2)
        .unwrap()
        .unwrap();
    let locators = store
        .application_table_sample_rows("session_locators", &["locator_value"], 2)
        .unwrap()
        .unwrap();
    assert_eq!(sessions[0]["pubkey"], "alive");
    assert_eq!(locators[0]["locator_value"], "alive-endpoint");
}

#[test]
fn session_context_persists_host_workspace_without_fabricating_channel_metadata() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&reg("pk", "pending-room", 100))
        .unwrap();
    store
        .set_session_context("pk", "pending-room", "workspace", "immediate-parent")
        .unwrap();

    let session = store.get_session("pk").unwrap().unwrap();
    assert_eq!(session.channel_h, "pending-room");
    assert_eq!(session.work_root, "workspace");
    assert_eq!(session.readiness_parent, "immediate-parent");
    assert_eq!(
        store
            .session_readiness_parent("pending-room")
            .unwrap()
            .as_deref(),
        Some("immediate-parent")
    );
    assert!(store.get_channel("pending-room").unwrap().is_none());
}

#[test]
fn table_samples_prefer_fresh_status_rows() {
    let store = Store::open_memory().unwrap();
    for (pubkey, updated_at, expiration) in [("old", 100, 100), ("fresh", 200, 300)] {
        store
            .upsert_status(&Status {
                pubkey: pubkey.into(),
                channel_h: "room".into(),
                slug: "agent".into(),
                title: String::new(),
                activity: String::new(),
                state: crate::session_state::SessionState::Idle,
                state_since: updated_at,
                last_seen: updated_at,
                updated_at,
                expiration,
            })
            .unwrap();
    }
    let rows = store
        .application_table_sample_rows("relay_status", &["pubkey"], 2)
        .unwrap()
        .unwrap();
    assert_eq!(rows[0]["pubkey"], "fresh");
}
