use super::*;
use crate::test_env::EnvGuard;

#[path = "tests/codex_named.rs"]
mod codex_named;
#[path = "tests/hermes.rs"]
mod hermes;

fn write(path: &std::path::Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn write_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    write(path, "#!/bin/sh\n");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn installed_codex_agent_resolves_without_agent_json() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let codex_home = home.path().join(".codex");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    env.set_var("CODEX_HOME", &codex_home);
    write(
        &mosaico_home.join("harnesses.json"),
        r#"{"codex-rpc":{"harness":"codex","transport":"app-server"}}"#,
    );
    write(
        &codex_home.join("agents/reviewer.toml"),
        "name='reviewer'\ndescription='Reviews code'\ndeveloper_instructions='Review carefully'",
    );
    write_executable(&home.path().join(".local/bin/codex"));
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;
    state.refresh_agent_catalog().unwrap();

    let source =
        resolve_agent_source(&state, "reviewer", &workspace, LaunchIntent::Managed).unwrap();
    assert_eq!(source.bundle, "codex-rpc");
    assert!(source.identity.per_session_key);
    assert!(source.identity.keys.is_none());
    assert!(matches!(
        source.native_agent,
        Some(NativeAgentActivation::CodexRoot(_))
    ));
    assert!(!mosaico_home.join("agents/reviewer.json").exists());
}

#[tokio::test]
async fn installed_opencode_agent_resolves_to_native_agent_argv() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    env.set_var("XDG_CONFIG_HOME", home.path().join(".config"));
    write(
        &mosaico_home.join("harnesses.json"),
        r#"{"opencode-pty":{"harness":"opencode","transport":"pty","args":["--verbose"]}}"#,
    );
    write(
        &home.path().join(".config/opencode/agents/new-profile.md"),
        "---\ndescription: Handles backend changes\n---\nWork carefully",
    );
    write_executable(&home.path().join(".opencode/bin/opencode"));
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;
    state.refresh_agent_catalog().unwrap();

    let source =
        resolve_agent_source(&state, "new-profile", &workspace, LaunchIntent::Managed).unwrap();
    assert_eq!(source.bundle, "opencode-pty");
    assert_eq!(
        source.command,
        ["opencode", "--verbose", "--agent", "new-profile"]
    );
    assert!(source.identity.per_session_key);
    assert!(source.identity.keys.is_none());
    assert!(!mosaico_home.join("agents/new-profile.json").exists());
}

#[tokio::test]
async fn interactive_generic_creates_pty_bundle_from_live_detection() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    std::fs::create_dir_all(home.path().join(".codex")).unwrap();
    write_executable(&home.path().join(".local/bin/codex"));
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;
    state.refresh_agent_catalog().unwrap();

    let source =
        resolve_agent_source(&state, "codex", &workspace, LaunchIntent::Interactive).unwrap();

    assert_eq!(source.identity.slug, "codex");
    assert_eq!(source.bundle, "codex-pty");
    assert_eq!(source.command, ["codex"]);
    let saved = HarnessesConfig::load().unwrap();
    assert_eq!(saved.get("codex-pty").unwrap().transport, Transport::Pty);
    assert!(!mosaico_home.join("agents/codex.json").exists());
}

#[test]
fn harness_resume_policy_ignores_stale_agent_binding() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    write(
        &mosaico_home.join("harnesses.json"),
        r#"{"claude-pty":{"harness":"claude-code","transport":"pty","args":["--dangerously-skip-permissions"]}}"#,
    );
    write(
        &mosaico_home.join("agents/developer.json"),
        r#"{"slug":"developer","created_at":1,"perSessionKey":true,"harness":"claude-code"}"#,
    );

    let source = resolve_harness_source(
        crate::session::Harness::ClaudeCode,
        "developer",
        None,
        LaunchIntent::Interactive,
    )
    .unwrap();

    assert_eq!(source.identity.slug, "developer");
    assert_eq!(source.bundle, "claude-pty");
    assert_eq!(source.command, ["claude", "--dangerously-skip-permissions"]);
}

#[test]
fn mapped_resume_prefers_its_recorded_pty_bundle() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    write(
        &mosaico_home.join("harnesses.json"),
        r#"{
          "claude-fast":{"harness":"claude-code","transport":"pty","args":["--fast"]},
          "claude-safe":{"harness":"claude-code","transport":"pty","args":["--safe"]}
        }"#,
    );

    let source = resolve_harness_source(
        crate::session::Harness::ClaudeCode,
        "agent1",
        Some("claude-safe"),
        LaunchIntent::Interactive,
    )
    .unwrap();

    assert_eq!(source.bundle, "claude-safe");
    assert_eq!(source.command, ["claude", "--safe"]);
}

#[tokio::test]
async fn invalid_same_named_agent_does_not_shadow_available_harness() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    write(
        &mosaico_home.join("harnesses.json"),
        r#"{"codex-pty":{"harness":"codex","transport":"pty","args":["--yolo"]}}"#,
    );
    write_executable(&home.path().join(".local/bin/codex"));
    crate::identity::add_local_agent(&mosaico_home, "codex", "codex", None, 10).unwrap();
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;

    let source =
        resolve_agent_source(&state, "codex", &workspace, LaunchIntent::Interactive).unwrap();

    assert_eq!(source.identity.slug, "codex");
    assert_eq!(source.bundle, "codex-pty");
    assert_eq!(source.command, ["codex", "--yolo"]);
}

#[tokio::test]
async fn conflict_combination_resolves_and_persists_selected_binding() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let codex_home = home.path().join(".codex");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    env.set_var("CODEX_HOME", &codex_home);
    write(
        &mosaico_home.join("harnesses.json"),
        r#"{
          "claude-pty":{"harness":"claude-code","transport":"pty"},
          "codex-pty":{"harness":"codex","transport":"pty"}
        }"#,
    );
    write(
        &codex_home.join("agents/writer.toml"),
        "name='writer'\ndescription='Writes'\ndeveloper_instructions='Write'",
    );
    write(
        &home.path().join(".claude/agents/writer.md"),
        "---\nname: writer\ndescription: Writes\n---\nWrite",
    );
    for executable in ["claude", "codex"] {
        write_executable(&home.path().join(".local/bin").join(executable));
    }
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;
    state.refresh_agent_catalog().unwrap();

    let source = resolve_agent_source(
        &state,
        "writer-codex",
        &workspace,
        LaunchIntent::Interactive,
    )
    .unwrap();

    assert_eq!(source.identity.slug, "writer");
    assert_eq!(source.bundle, "codex-pty");
    assert_eq!(
        source.retired_advertisements,
        ["writer-claude", "writer-codex"]
    );
    let saved = crate::identity::agent_launch_config(&mosaico_home, "writer").unwrap();
    assert_eq!(saved.harness, "codex-pty");
    assert!(saved.profile.is_none());
}

#[tokio::test]
async fn managed_generic_creates_preferred_rpc_bundle() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    std::fs::create_dir_all(home.path().join(".codex")).unwrap();
    write_executable(&home.path().join(".local/bin/codex"));
    let workspace = home.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = DaemonState::new_for_test().await;
    state.refresh_agent_catalog().unwrap();

    let source = resolve_agent_source(&state, "codex", &workspace, LaunchIntent::Managed).unwrap();

    assert_eq!(source.bundle, "codex-app-server");
    assert_eq!(
        source.transport.kind(),
        crate::session_host::transport::TransportKind::AppServer
    );
    let saved = HarnessesConfig::load().unwrap();
    assert_eq!(
        saved.get("codex-app-server").unwrap().transport,
        Transport::AppServer
    );
}

#[test]
fn goose_uses_pty_for_interactive_and_acp_for_managed_launches() {
    assert_eq!(
        desired_transport(
            crate::session::Harness::Goose,
            LaunchIntent::Interactive,
            false
        )
        .unwrap(),
        Transport::Pty
    );
    assert_eq!(
        desired_transport(crate::session::Harness::Goose, LaunchIntent::Managed, false).unwrap(),
        Transport::Acp
    );
    assert!(
        desired_transport(crate::session::Harness::Goose, LaunchIntent::Managed, true).is_err()
    );
}
