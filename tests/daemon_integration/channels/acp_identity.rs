use super::*;
use nostr_sdk::prelude::Keys;
use std::time::Duration;

#[test]
fn acp_agent_receives_the_signer_matching_its_assigned_pubkey() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        r#"{"test-acp":{"harness":"opencode","transport":"acp"}}"#,
    )
    .unwrap();
    let agent = "acp-agent-nsec";
    mosaico::identity::add_local_agent(home.dir.path(), agent, "test-acp", None, 1).unwrap();
    let channel = format!("acp-agent-nsec-{}", std::process::id());
    let work_dir = home.dir.path().join(&channel);
    std::fs::create_dir_all(&work_dir).unwrap();
    std::fs::write(
        home.dir.path().join("workspaces.json"),
        serde_json::json!({&channel: &work_dir}).to_string(),
    )
    .unwrap();

    let out = run_cli_with_env_in_dir(
        &home,
        &["launch", agent, "--workspace", &channel],
        &[],
        &work_dir,
    );
    assert!(
        out.status.success(),
        "ACP launch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let capture = home.dir.path().join("captured-acp-identity");
    assert!(
        wait_until(Duration::from_secs(10), || capture.exists()),
        "ACP harness did not capture its assigned identity"
    );

    let captured = std::fs::read_to_string(&capture).unwrap();
    let mut lines = captured.lines();
    let pubkey = lines.next().expect("captured pubkey");
    let nsec = lines.next().expect("captured nsec");
    assert_eq!(Keys::parse(nsec).unwrap().public_key().to_hex(), pubkey);
    assert!(Store::open(&home.store_path())
        .unwrap()
        .get_session(pubkey)
        .unwrap()
        .is_some());

    stop_daemon(&home);
}
