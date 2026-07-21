use super::*;

#[tokio::test]
async fn managed_hermes_creates_acp_bundle() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    write_executable(&home.path().join(".local/bin/hermes"));
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;
    state.refresh_agent_catalog().unwrap();

    let source = resolve_agent_source(&state, "hermes", &workspace, LaunchIntent::Managed).unwrap();

    assert_eq!(source.bundle, "hermes-acp");
    assert_eq!(source.command, ["hermes", "acp"]);
    assert_eq!(
        source.transport.kind(),
        crate::session_host::transport::TransportKind::Acp
    );
    let saved = HarnessesConfig::load().unwrap();
    assert_eq!(saved.get("hermes-acp").unwrap().transport, Transport::Acp);
}
