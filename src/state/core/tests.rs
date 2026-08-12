use super::*;

fn reg(pubkey: &str, channel: &str, now: u64) -> RegisterSession {
    RegisterSession {
        pubkey: pubkey.into(),
        observed_harness: "codex".into(),
        agent_slug: "agent".into(),
        launch_channel_h: channel.into(),
        work_root: channel.into(),
        child_pid: None,
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
    let mut registration = reg("pk", "pending-room", 100);
    registration.work_root = "workspace".into();
    store.reserve_hook_session_for_test(&registration).unwrap();
    store
        .set_session_readiness_parent("pk", "immediate-parent")
        .unwrap();

    let session = store.get_session("pk").unwrap().unwrap();
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
fn registered_work_root_cannot_change_on_relaunch() {
    let store = Store::open_memory().unwrap();
    let mut registration = reg("pk", "launch", 100);
    registration.work_root = "workspace".into();
    let generation = store.reserve_hook_session_for_test(&registration).unwrap();
    store
        .mark_runtime_stopped_if_generation("pk", generation, StopReason::Crash, 101)
        .unwrap();

    registration.work_root = "other-workspace".into();
    registration.now = 102;
    let error = store
        .reserve_hook_session_for_test(&registration)
        .unwrap_err();

    assert!(error.to_string().contains("immutable work root"));
    assert_eq!(
        store.get_session("pk").unwrap().unwrap().work_root,
        "workspace"
    );
}
