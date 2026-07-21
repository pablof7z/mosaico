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

#[tokio::test]
async fn managed_native_hermes_profile_uses_global_selector_before_acp() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let hermes_home = home.path().join(".hermes");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    env.set_var("HERMES_HOME", &hermes_home);
    write(
        &hermes_home.join("profiles/reviewer/profile.yaml"),
        "description: Reviews completed work.\n",
    );
    write_executable(&home.path().join(".local/bin/hermes"));
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;
    state.refresh_agent_catalog().unwrap();

    let source =
        resolve_agent_source(&state, "reviewer", &workspace, LaunchIntent::Managed).unwrap();

    assert_eq!(source.bundle, "hermes-acp");
    assert_eq!(source.command, ["hermes", "--profile", "reviewer", "acp"]);
    assert_eq!(
        source.native_agent,
        Some(NativeAgentActivation::NativeSelector {
            name: "reviewer".into()
        })
    );
    assert!(!mosaico_home.join("agents/reviewer.json").exists());
}

#[test]
fn native_hermes_profiles_admit_interactive_and_managed_transports() {
    assert_eq!(
        desired_transport(
            crate::session::Harness::Hermes,
            LaunchIntent::Interactive,
            true,
        )
        .unwrap(),
        Transport::Pty
    );
    assert_eq!(
        desired_transport(crate::session::Harness::Hermes, LaunchIntent::Managed, true).unwrap(),
        Transport::Acp
    );
}
