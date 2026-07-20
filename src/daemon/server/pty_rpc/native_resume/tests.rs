use super::*;

#[tokio::test]
async fn running_non_pty_session_refuses_a_second_process() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|store| {
            store.reserve_hook_session_for_test(&crate::state::RegisterSession {
                pubkey: "mapped-pubkey".into(),
                observed_harness: "codex".into(),
                agent_slug: "agent1".into(),
                channel_h: "mosaico".into(),
                child_pid: Some(42),
                transcript_path: None,
                now: 1,
            })
        })
        .unwrap();
    let session = state
        .with_store(|store| store.get_session("mapped-pubkey"))
        .unwrap()
        .unwrap();

    let error = resume_mapped(&state, &session, "native-id")
        .await
        .unwrap_err();

    assert!(error.to_string().contains("already running"));
    assert!(error.to_string().contains("mosaico sessions"));
}
