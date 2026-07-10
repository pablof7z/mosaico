use super::*;
use crate::state::RegisterSession;

fn caller_session(state: &Arc<DaemonState>, channels: &[&str]) -> crate::state::Session {
    state.with_store(|s| {
        s.upsert_session_row(
            "caller-session",
            &RegisterSession {
                harness: "codex".to_string(),
                external_id_kind: "harness_session".to_string(),
                external_id: "caller-session".to_string(),
                agent_pubkey: "caller-pubkey".to_string(),
                agent_slug: "codex".to_string(),
                channel_h: channels.first().copied().unwrap_or("project1").to_string(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now: 1,
            },
        )
        .unwrap();
        for (idx, channel) in channels.iter().enumerate().skip(1) {
            s.join_session_channel("caller-session", channel, 2 + idx as u64)
                .unwrap();
        }
        s.get_session("caller-session").unwrap().unwrap()
    })
}

#[tokio::test]
async fn route_channel_is_first_requested_channel_shared_with_caller() {
    let state = DaemonState::new_for_test().await;
    let caller = caller_session(&state, &["project1", "project1.bug-123"]);

    let route = first_shared_channel(
        &state,
        &caller,
        &["project2.qa".to_string(), "project1.bug-123".to_string()],
    )
    .unwrap();

    assert_eq!(route, "project1.bug-123");
}

#[tokio::test]
async fn route_channel_failure_lists_channels_the_caller_is_active_on() {
    let state = DaemonState::new_for_test().await;
    let caller = caller_session(&state, &["project1", "project1.dev"]);

    let err = first_shared_channel(&state, &caller, &["project2".to_string()])
        .unwrap_err()
        .to_string();

    assert!(err.contains("you need to specify a channel you're active on:"));
    assert!(err.contains("project1"));
    assert!(err.contains("project1.dev") || err.contains("@project1"));
}
