use super::*;
use crate::test_env::EnvGuard;

fn write(path: &std::path::Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn isolated_home() -> (tempfile::TempDir, std::path::PathBuf, EnvGuard) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("mosaico");
    let mut env = EnvGuard::set("MOSAICO_HOME", &home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", root.path());
    (root, home, env)
}

#[test]
fn interactive_launch_uses_pty_without_implicit_args() {
    let (_root, _home, _env) = isolated_home();
    let source = resolve_harness_source(
        crate::session::Harness::Codex,
        "codex",
        None,
        LaunchIntent::Interactive,
    )
    .unwrap();
    assert_eq!(source.command, ["codex"]);
    assert_eq!(
        source.transport.kind(),
        crate::session_host::transport::TransportKind::Pty
    );
    assert_eq!(source.preset, None);
}

#[test]
fn one_harness_and_preset_realize_per_launch_transport() {
    let (_root, home, _env) = isolated_home();
    write(
        &home.join("presets.json"),
        r#"{"unrestricted":{"codex":{"pty":["--yolo"],"app-server":["--server-arg"]}}}"#,
    );
    crate::identity::add_local_agent(&home, "reviewer", "codex", None, Some("unrestricted"), 1)
        .unwrap();

    let interactive = resolve_harness_source(
        crate::session::Harness::Codex,
        "reviewer",
        None,
        LaunchIntent::Interactive,
    )
    .unwrap();
    assert_eq!(interactive.command, ["codex", "--yolo"]);

    let managed = resolve_harness_source(
        crate::session::Harness::Codex,
        "reviewer",
        None,
        LaunchIntent::Managed,
    )
    .unwrap();
    assert_eq!(managed.command, ["codex", "app-server", "--server-arg"]);
    assert_eq!(managed.preset.as_deref(), Some("unrestricted"));
}

#[test]
fn missing_selected_preset_fails_loudly() {
    let (_root, home, _env) = isolated_home();
    crate::identity::add_local_agent(&home, "reviewer", "codex", None, Some("missing"), 1).unwrap();
    let error = resolve_harness_source(
        crate::session::Harness::Codex,
        "reviewer",
        None,
        LaunchIntent::Interactive,
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("missing"));
}

#[test]
fn managed_resume_can_preserve_admitted_transport() {
    let (_root, _home, _env) = isolated_home();
    let source = resolve_harness_source(
        crate::session::Harness::Codex,
        "reviewer",
        Some("pty"),
        LaunchIntent::Managed,
    )
    .unwrap();
    assert_eq!(
        source.transport.kind(),
        crate::session_host::transport::TransportKind::Pty
    );
}
