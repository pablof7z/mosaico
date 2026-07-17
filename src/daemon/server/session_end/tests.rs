use super::*;

fn register(
    state: &Arc<DaemonState>,
    pubkey: &str,
    admitted_transport: &str,
    child_pid: i32,
) -> crate::state::Session {
    state
        .with_store(|store| {
            let registration = crate::state::RegisterSession {
                pubkey: pubkey.into(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                channel_h: "root".into(),
                child_pid: Some(child_pid),
                transcript_path: None,
                now: 1,
            };
            if admitted_transport.is_empty() {
                store.reserve_hook_session_for_test(&registration)?;
            } else {
                store.reserve_session_with_facts(
                    &registration,
                    &crate::state::AdmittedRuntimeFacts {
                        observed_harness: "codex".into(),
                        claimed_harness: String::new(),
                        bundle: format!("codex-{admitted_transport}"),
                        transport: admitted_transport.into(),
                        endpoint_provenance: "launch".into(),
                    },
                )?;
            }
            store
                .get_session(pubkey)?
                .ok_or_else(|| anyhow::anyhow!("registered session disappeared"))
        })
        .unwrap()
}

#[tokio::test]
async fn admitted_hosted_session_without_locator_refuses_pid_fallback() {
    let state = DaemonState::new_for_test().await;
    for transport in ["pty", "acp", "app-server"] {
        let rec = register(
            &state,
            &format!("pk-missing-{transport}"),
            transport,
            std::process::id() as i32,
        );
        let error = stop_local_process(&state, &rec).await.unwrap_err();
        assert!(
            error.to_string().contains("refusing PID fallback"),
            "{error:#}"
        );
    }
}

#[tokio::test]
async fn native_process_without_admitted_transport_keeps_pid_fallback() {
    let state = DaemonState::new_for_test().await;
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let rec = register(&state, "pk-native-process", "", child.id() as i32);

    let result = stop_local_process(&state, &rec).await;
    if result.is_err() {
        let _ = child.kill();
    }
    assert_eq!(result.unwrap(), format!("pid={}", child.id()));
    let status = child.wait().unwrap();
    assert!(!status.success());
}
