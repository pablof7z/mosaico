use super::*;
use crate::state::RegisterSession;
use serde_json::json;

const SELF_PK: &str = "self-pubkey";
const PEER_PK: &str = "peer-pubkey";

#[tokio::test]
async fn agent_who_renders_full_snapshot_after_cursor_advanced() {
    let state = DaemonState::new_for_test().await;
    let session_id = state.with_store(|s| {
        s.upsert_channel("root", "main", "Root room", "", 1)
            .unwrap();
        s.upsert_channel("task", "task", "Task room", "root", 2)
            .unwrap();
        s.replace_channel_members("root", &[SELF_PK.into(), PEER_PK.into()], 1)
            .unwrap();
        s.upsert_profile(SELF_PK, "coder", "coder", "test-host", false, 1)
            .unwrap();
        s.upsert_profile(PEER_PK, "reviewer", "reviewer", "test-host", false, 1)
            .unwrap();
        let session_id = s
            .register_session(&RegisterSession {
                harness: "codex".into(),
                external_id_kind: "pty_session".into(),
                external_id: "pty-1".into(),
                agent_pubkey: SELF_PK.into(),
                agent_slug: "coder".into(),
                channel_h: "root".into(),
                child_pid: Some(42),
                transcript_path: None,
                resume_id: String::new(),
                now: 10,
            })
            .unwrap();
        s.apply_cursor_projection(&session_id, 200).unwrap();
        session_id
    });

    let out = rpc_who(&state, &json!({ "pty_session": "pty-1" })).unwrap();
    let fabric = out
        .get("fabric")
        .and_then(|v| v.as_str())
        .expect("who should include fabric context");

    assert!(fabric.contains("<members>"), "got:\n{fabric}");
    assert!(fabric.contains("id=\"root.task\""), "got:\n{fabric}");
    let handle = "coder";
    assert!(
        fabric.contains(&format!("<agent name=\"@{handle}\"")),
        "caller must stay typed as an agent on a cold status cache:\n{fabric}"
    );
    assert!(
        !fabric.contains("<no-new-activity"),
        "explicit who must not render as a quiet delta:\n{fabric}"
    );

    let seen_cursor = state.with_store(|s| {
        s.get_session(&session_id)
            .unwrap()
            .expect("session should still exist")
            .seen_cursor
    });
    assert!(
        seen_cursor >= 200,
        "who should not regress the stored cursor: {seen_cursor}"
    );
}
