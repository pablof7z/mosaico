use super::*;

#[tokio::test]
async fn native_kimi_profile_uses_pty_agent_selector() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let kimi_home = home.path().join(".kimi-code");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    env.set_var("KIMI_CODE_HOME", &kimi_home);
    write(
        &kimi_home.join("agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews completed work\n---\nReview",
    );
    write_executable(&home.path().join(".local/bin/kimi"));
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;
    state.refresh_agent_catalog().unwrap();

    let source =
        resolve_agent_source(&state, "reviewer", &workspace, LaunchIntent::Managed).unwrap();

    assert_eq!(source.bundle, "kimi-pty");
    assert_eq!(source.command, ["kimi", "--agent", "reviewer"]);
    assert_eq!(
        source.transport.kind(),
        crate::session_host::transport::TransportKind::Pty
    );
    assert_eq!(
        source.native_agent,
        Some(NativeAgentActivation::NativeSelector {
            name: "reviewer".into()
        })
    );
    assert!(!mosaico_home.join("agents/reviewer.json").exists());
}

#[test]
fn uses_pty_for_interactive_and_acp_for_managed_launches() {
    assert_eq!(
        desired_transport(
            crate::session::Harness::Kimi,
            LaunchIntent::Interactive,
            false
        )
        .unwrap(),
        Transport::Pty
    );
    assert_eq!(
        desired_transport(crate::session::Harness::Kimi, LaunchIntent::Managed, false).unwrap(),
        Transport::Acp
    );
    assert_eq!(
        desired_transport(crate::session::Harness::Kimi, LaunchIntent::Managed, true).unwrap(),
        Transport::Pty
    );
}
