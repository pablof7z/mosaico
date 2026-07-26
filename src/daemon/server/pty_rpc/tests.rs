use super::*;

#[tokio::test]
async fn resumable_sessions_expose_only_full_public_channel_paths() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|store| {
            store.upsert_channel("root", "general", "", "", 1)?;
            store.upsert_channel("opaque-child", "development", "", "root", 2)?;
            let generation =
                store.reserve_hook_session_for_test(&crate::state::RegisterSession {
                    pubkey: "resumable-pubkey".into(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    launch_channel_h: "root".into(),
                    work_root: "root".into(),
                    child_pid: None,
                    now: 3,
                })?;
            store.grant_session_route("resumable-pubkey", "opaque-child", 4)?;
            store.set_native_resume_locator("resumable-pubkey", "codex", "native-resume", 5)?;
            store.mark_runtime_stopped_if_generation(
                "resumable-pubkey",
                generation,
                crate::state::StopReason::Crash,
                6,
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();

    let response = rpc_pty_resumable(&state).await.unwrap();
    let row = &response["resumable"][0];
    assert_eq!(row["work_root"], "/root");
    assert_eq!(
        row["channels"],
        serde_json::json!(["/root", "/root/development"])
    );
    assert!(!response.to_string().contains("opaque-child"));
}
